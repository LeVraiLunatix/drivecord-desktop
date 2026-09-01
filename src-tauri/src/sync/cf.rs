//! The Cloud Filter side: registers the sync root, connects the callback
//! filter, and runs the reconcile loop. Everything here lives on one
//! dedicated OS thread (`run_worker`, spawned from `SyncEngine::start`) —
//! the `Connection` the crate hands back is a thread-local kind of resource,
//! so it never gets moved elsewhere. The rest of the app only ever sees
//! `EngineCmd`s in and `SyncStatus` snapshots out.

use std::{future::Future, path::Path, time::Duration};

use cloud_filter::{
    error::{CResult, CloudErrorKind},
    filter::{info, ticket, Filter, Request},
    root::{HydrationType, PopulationType, SecurityId, Session, SyncRootId, SyncRootIdBuilder, SyncRootInfo},
    utility::WriteAt,
};

use super::{api, crypto, discord, mirror, upload, EngineCmd, EngineCtx, RECONCILE_INTERVAL_SECS};

pub(crate) fn run_worker(ctx: EngineCtx, mut cmd_rx: tokio::sync::mpsc::Receiver<EngineCmd>) {
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            ctx.set_error(format!("runtime tokio : {e}"));
            return;
        }
    };
    rt.block_on(async_main(ctx, &mut cmd_rx));
}

async fn async_main(ctx: EngineCtx, cmd_rx: &mut tokio::sync::mpsc::Receiver<EngineCmd>) {
    let root = ctx.root.clone();
    if let Err(e) = ensure_registered(&root) {
        ctx.set_error(format!("enregistrement de la racine de synchro : {e}"));
        return;
    }

    let handle = tokio::runtime::Handle::current();
    let filter = DriveFilter(ctx.clone());
    let connection = match Session::new().connect_async(&root, filter, move |fut| handle.block_on(fut)) {
        Ok(c) => c,
        Err(e) => {
            ctx.set_error(format!("connexion à l'API Cloud Files : {e}"));
            return;
        }
    };

    mirror::reconcile_all(&ctx).await;

    let mut ticker = tokio::time::interval(Duration::from_secs(RECONCILE_INTERVAL_SECS));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await; // consume the immediate first tick — we just reconciled above

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                mirror::reconcile_all(&ctx).await;
            }
            cmd = cmd_rx.recv() => match cmd {
                Some(EngineCmd::ReconcileNow) => mirror::reconcile_all(&ctx).await,
                Some(EngineCmd::Stop) | None => break,
            }
        }
    }

    drop(connection);
}

/// Registers the sync root once (idempotent — the id is deterministic per
/// provider+user). Changing the chosen folder after the first registration
/// isn't handled here; disable sync before picking a different folder.
fn ensure_registered(root: &Path) -> Result<SyncRootId, String> {
    let id = SyncRootIdBuilder::new(super::PROVIDER_NAME)
        .user_security_id(SecurityId::current_user().map_err(|e| e.to_string())?)
        .build();
    if !id.is_registered().map_err(|e| e.to_string())? {
        id.register(
            SyncRootInfo::default()
                .with_display_name(super::DISPLAY_NAME)
                .with_hydration_type(HydrationType::Full)
                .with_population_type(PopulationType::Full)
                .with_path(root)
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(id)
}

/// The Cloud Filter callback implementation. Cheap to clone (`EngineCtx` is
/// all `Arc`/`Client` handles), so callbacks that need to do network I/O just
/// clone it and `tokio::spawn` rather than blocking the CfAPI thread.
struct DriveFilter(EngineCtx);

/// A file placeholder's identity blob: `driveId\0fileId` (UTF-8). Kept tiny —
/// CfAPI caps blobs at 4 KiB — everything else is looked up from the shared
/// `FileIndex`, refreshed on every reconcile pass.
fn parse_blob(blob: &[u8]) -> Option<(String, String)> {
    let s = std::str::from_utf8(blob).ok()?;
    let (drive_id, file_id) = s.split_once('\u{0}')?;
    Some((drive_id.to_string(), file_id.to_string()))
}

impl Filter for DriveFilter {
    fn fetch_data(
        &self,
        request: Request,
        ticket: ticket::FetchData,
        _info: info::FetchData,
    ) -> impl Future<Output = CResult<()>> {
        let ctx = self.0.clone();
        async move {
            let Some((drive_id, file_id)) = parse_blob(request.file_blob()) else {
                return Err(CloudErrorKind::InvalidRequest);
            };
            let (secrets, entry) = {
                let idx = ctx.index.read();
                let secrets = idx.drives.get(&drive_id).cloned();
                let entry = idx.files.get(&file_id).map(|(_, e)| e.clone());
                (secrets, entry)
            };
            let (Some(secrets), Some(entry)) = (secrets, entry) else {
                return Err(CloudErrorKind::InvalidRequest);
            };

            let ciphertext = discord::download_all(&ctx.http, &secrets.webhook_url, &entry.chunks)
                .await
                .map_err(|_| CloudErrorKind::NetworkUnavailable)?;
            let plain = match entry.enc_iv.as_deref() {
                Some(iv) => crypto::decrypt(&secrets.enc_key, iv, &ciphertext)
                    .map_err(|_| CloudErrorKind::ValidationFailed)?,
                None => ciphertext,
            };

            ticket.write_at(&plain, 0).map_err(|_| CloudErrorKind::InvalidRequest)?;
            Ok(())
        }
    }

    fn delete(
        &self,
        request: Request,
        ticket: ticket::Delete,
        info: info::Delete,
    ) -> impl Future<Output = CResult<()>> {
        let ctx = self.0.clone();
        async move {
            // Folder deletions aren't propagated in this pass — approve the
            // local delete but leave the cloud side untouched.
            if !info.is_directory() {
                if let Some((drive_id, file_id)) = parse_blob(request.file_blob()) {
                    tokio::spawn(async move {
                        let body = api::PatchFileBody { trashed: Some(true), ..Default::default() };
                        let _ = api::Api::new(ctx.http.clone(), ctx.token.clone())
                            .patch_file(&drive_id, &file_id, &body)
                            .await;
                    });
                }
            }
            ticket.pass().map_err(|_| CloudErrorKind::InvalidRequest)?;
            Ok(())
        }
    }

    fn rename(
        &self,
        request: Request,
        ticket: ticket::Rename,
        info: info::Rename,
    ) -> impl Future<Output = CResult<()>> {
        let ctx = self.0.clone();
        async move {
            ticket.pass().map_err(|_| CloudErrorKind::InvalidRequest)?;

            if !info.source_in_scope() || !info.target_in_scope() {
                return Ok(());
            }
            let Some((drive_id, file_id)) = parse_blob(request.file_blob()) else {
                return Ok(());
            };
            let dest = info.target_path();
            let new_name = dest.file_name().and_then(|n| n.to_str()).map(str::to_string);
            let new_parent_id = dest
                .parent()
                .and_then(|p| ctx.index.read().path_to_folder.get(p).map(|(_, id)| id.clone()));

            tokio::spawn(async move {
                let body = api::PatchFileBody {
                    filename: new_name,
                    parent_id: new_parent_id,
                    ..Default::default()
                };
                let _ = api::Api::new(ctx.http.clone(), ctx.token.clone())
                    .patch_file(&drive_id, &file_id, &body)
                    .await;
            });
            Ok(())
        }
    }

    /// Fires for any change under the sync root (crate-managed
    /// `ReadDirectoryChangesW` watcher). Used to catch real files the user
    /// drops in — anything not already a known placeholder gets uploaded.
    fn state_changed(&self, changes: Vec<std::path::PathBuf>) -> impl Future<Output = ()> {
        let ctx = self.0.clone();
        async move {
            for path in changes {
                if ctx.index.read().path_to_file.contains_key(&path) {
                    continue; // already a known placeholder / synced file
                }
                if !path.is_file() {
                    continue;
                }
                let Some(parent) = path.parent() else { continue };
                let Some((drive_id, folder_id)) = ctx.index.read().path_to_folder.get(parent).cloned() else {
                    continue; // not under a recognised drive folder
                };
                let Some(secrets) = ctx.index.read().drives.get(&drive_id).cloned() else {
                    continue;
                };
                let ctx2 = ctx.clone();
                tokio::spawn(async move {
                    try_upload_settled(ctx2, drive_id, secrets, folder_id, path).await;
                });
            }
        }
    }
}

/// Wait for a newly-seen file's size to stop changing (the user may still be
/// copying it in), then upload it. Guards against duplicate triggers for the
/// same path via `EngineCtx::uploading`.
async fn try_upload_settled(
    ctx: EngineCtx,
    drive_id: String,
    secrets: super::DriveSecrets,
    folder_id: String,
    path: std::path::PathBuf,
) {
    {
        let mut inflight = ctx.uploading.write();
        if !inflight.insert(path.clone()) {
            return; // already being handled by another event for the same path
        }
    }

    let mut last_size = None;
    for _ in 0..8 {
        tokio::time::sleep(Duration::from_millis(700)).await;
        let Ok(meta) = tokio::fs::metadata(&path).await else {
            ctx.uploading.write().remove(&path);
            return; // gone already (renamed/deleted mid-debounce)
        };
        let size = meta.len();
        if Some(size) == last_size {
            break;
        }
        last_size = Some(size);
    }

    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("fichier")
        .to_string();

    match upload::upload_new_file(&ctx, &drive_id, &secrets, &folder_id, &path, &filename).await {
        Ok(file_id) => {
            ctx.index.write().path_to_file.insert(path.clone(), (drive_id, file_id));
        }
        Err(e) => eprintln!("sync: échec de l'envoi de {path:?} : {e}"),
    }

    ctx.uploading.write().remove(&path);
    ctx.touch_status();
}
