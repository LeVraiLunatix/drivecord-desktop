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
    let icon = ctx
        .config_path
        .parent()
        .and_then(super::folder_style::ensure_icon_file);
    let sync_root_id = match ensure_registered(&root, icon.as_deref()) {
        Ok(id) => id,
        Err(e) => {
            ctx.set_error(format!("enregistrement de la racine de synchro : {e}"));
            return;
        }
    };

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

    // Disk -> cloud: watch the root for files the user drops in.
    let (watch_tx, mut watch_rx) = tokio::sync::mpsc::unbounded_channel::<std::path::PathBuf>();
    let _watcher = match super::watcher::spawn(&root, watch_tx) {
        Ok(w) => Some(w),
        Err(e) => {
            eprintln!("sync: watcher fs indisponible ({e}) — l'upload auto ne marchera pas");
            None
        }
    };
    let watch_ctx = ctx.clone();
    let watch_task = tokio::spawn(async move {
        while let Some(path) = watch_rx.recv().await {
            consider_local_path(&watch_ctx, path);
        }
    });

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

    watch_task.abort();
    drop(_watcher);
    drop(connection);
    // Sync stopped (user disabled it, or the app is quitting) — deregister so
    // the folder becomes a plain, deletable directory again. It's re-registered
    // on the next `start()`.
    let _ = sync_root_id.unregister();
}

/// Registers the sync root once (idempotent — the id is deterministic per
/// provider+user). Changing the chosen folder after the first registration
/// isn't handled here; disable sync before picking a different folder.
fn ensure_registered(root: &Path, icon_path: Option<&Path>) -> Result<SyncRootId, String> {
    let id = SyncRootIdBuilder::new(super::PROVIDER_NAME)
        .user_security_id(SecurityId::current_user().map_err(|e| e.to_string())?)
        .build();

    // Windows refuses a registration with an empty icon resource. Prefer the
    // brand folder icon; fall back to the app exe, then a generic shell icon.
    let icon = icon_path
        .map(|p| format!("{},0", p.display()))
        .or_else(|| std::env::current_exe().ok().map(|p| format!("{},0", p.display())))
        .unwrap_or_else(|| "%SystemRoot%\\system32\\imageres.dll,-1043".to_string());

    let info = SyncRootInfo::default()
        .with_display_name(super::DISPLAY_NAME)
        .with_icon(icon)
        .with_version(env!("CARGO_PKG_VERSION"))
        .with_hydration_type(HydrationType::Full)
        // AlwaysFull: the OS assumes we've populated everything and never calls
        // `fetch_placeholders`. `Full` in this crate is the *on-demand* mode,
        // which would need a `fetch_placeholders` impl (we populate eagerly
        // from the mirror loop instead).
        .with_population_type(PopulationType::AlwaysFull)
        .with_path(root)
        .map_err(|e| e.to_string())?;

    // Start from a clean slate every run: deregister the old registration (no-op
    // if absent) then re-register with the current icon / version / policy.
    let _ = id.unregister();
    id.register(info).map_err(|e| e.to_string())?;
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

            // Total the OS expects on disk (the decrypted size). Used only to
            // drive the progress bar / keep the 60s callback timer alive.
            let logical = if entry.enc_iv.is_some() {
                entry.size.saturating_sub(16)
            } else {
                entry.size
            };

            // Download phase counts for ~90% of the progress bar, the disk
            // write for the last ~10%.
            let ciphertext = discord::download_all(
                &ctx.http,
                &secrets.webhook_url,
                &entry.chunks,
                16,
                |done, total| {
                    let completed = (logical as u128 * 9 * done as u128
                        / (10 * total.max(1) as u128)) as u64;
                    let _ = ticket.report_progress(logical, completed);
                },
            )
            .await
            .map_err(|_| CloudErrorKind::NetworkUnavailable)?;

            let plain = match entry.enc_iv.as_deref() {
                Some(iv) => crypto::decrypt(&secrets.enc_key, iv, &ciphertext)
                    .map_err(|_| CloudErrorKind::ValidationFailed)?,
                None => ciphertext,
            };

            // Write in ~8 MiB blocks, pinging progress between each — a single
            // 200 MB CfExecute stalls Explorer and risks the 60s callback
            // timeout. The final block ends exactly on the logical size.
            const BLOCK: usize = 8 * 1024 * 1024;
            let total = plain.len();
            let mut offset = 0usize;
            while offset < total {
                let end = (offset + BLOCK).min(total);
                ticket
                    .write_at(&plain[offset..end], offset as u64)
                    .map_err(|_| CloudErrorKind::InvalidRequest)?;
                offset = end;
                let completed =
                    (logical as u128 * (9 * total as u128 + offset as u128) / (10 * total.max(1) as u128)) as u64;
                let _ = ticket.report_progress(logical, completed.min(logical));
            }
            if total == 0 {
                ticket.write_at(&[], 0).map_err(|_| CloudErrorKind::InvalidRequest)?;
            }
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

    /// The crate's own watcher only reports attribute changes (pin/unpin) —
    /// new files are caught by our `notify` watcher (see `watcher.rs`), which
    /// also calls `consider_local_path`.
    fn state_changed(&self, changes: Vec<std::path::PathBuf>) -> impl Future<Output = ()> {
        let ctx = self.0.clone();
        async move {
            for path in changes {
                consider_local_path(&ctx, path);
            }
        }
    }
}

/// Is this path a placeholder stub we (or the OS) created but that has no real
/// data on disk yet? Such files must never be treated as "new local content".
fn is_placeholder_stub(path: &std::path::Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
    const RECALL_ON_OPEN: u32 = 0x0004_0000;
    std::fs::metadata(path)
        .map(|m| m.file_attributes() & (RECALL_ON_DATA_ACCESS | RECALL_ON_OPEN) != 0)
        .unwrap_or(false)
}

/// Decide whether `path` is a brand-new real file the user dropped in and, if
/// so, kick off its upload. Safe to call for any path under the sync root.
pub(crate) fn consider_local_path(ctx: &EngineCtx, path: std::path::PathBuf) {
    if ctx.index.read().path_to_file.contains_key(&path) {
        return; // already a known placeholder / synced file
    }
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case(super::folder_style::DESKTOP_INI))
    {
        return; // our own folder-icon marker
    }
    if !path.is_file() || is_placeholder_stub(&path) {
        return;
    }
    let Some(parent) = path.parent() else { return };
    let (drive_id, folder_id) = {
        let idx = ctx.index.read();
        match idx.path_to_folder.get(parent).cloned() {
            Some(v) => v,
            None => return, // not under a recognised drive folder
        }
    };
    let Some(secrets) = ctx.index.read().drives.get(&drive_id).cloned() else {
        return;
    };
    let ctx = ctx.clone();
    tokio::spawn(async move {
        try_upload_settled(ctx, drive_id, secrets, folder_id, path).await;
    });
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
