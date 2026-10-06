//! Cloud -> disk reconciliation: fetch every drive's tree and make sure every
//! folder exists as a real directory and every file exists as an in-sync
//! placeholder. Runs on the sync worker thread, on an interval and on demand.

use std::{collections::HashMap, path::PathBuf};

use cloud_filter::{metadata::Metadata, placeholder_file::PlaceholderFile, utility::FileTime};

use super::{api, e2ee, keystore, DriveSecrets, DriveStatus, EngineCtx, FileIndex};

/// Windows forbids these in a path segment; also trim trailing dots/spaces
/// (Explorer silently strips them, which would break our identity mapping).
pub(crate) fn sanitize(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    while out.ends_with('.') || out.ends_with(' ') {
        out.pop();
    }
    if out.is_empty() {
        out = "_".to_string();
    }
    if out.len() > 200 {
        out.truncate(200);
    }
    out
}

fn filetime_from_unix_ms(ms: i64) -> Option<FileTime> {
    FileTime::from_unix_time(ms / 1000).ok()
}

pub async fn reconcile_all(ctx: &EngineCtx) {
    let cfg = ctx.config();
    let token = &ctx.token;
    let api = api::Api::new(ctx.http.clone(), token.clone());

    let webhooks = match api.list_webhooks().await {
        Ok(w) => w,
        Err(e) => {
            ctx.set_error(format!("Impossible de lister les drives : {e}"));
            return;
        }
    };

    {
        let mut s = ctx.status.write();
        s.state = "syncing".into();
    }

    let mut drive_statuses = Vec::new();
    let mut new_index = FileIndex::default();

    let icon = ctx
        .config_path
        .parent()
        .and_then(super::folder_style::ensure_icon_file);

    // The registration's `with_icon` only styles the navigation-pane entry —
    // give the physical sync root folder the same icon via desktop.ini too.
    if let Some(icon) = icon.as_deref() {
        super::folder_style::apply_icon(&ctx.root, icon);
    }

    let drive_keys = ctx
        .config_path
        .parent()
        .map(keystore::load)
        .unwrap_or_default();

    for wh in &webhooks {
        let e2ee_drive = wh.e2ee_version.unwrap_or(0) >= 1;
        let dk = if e2ee_drive {
            drive_keys.get(&wh.drive_id).copied()
        } else {
            None
        };
        let legacy_key = if e2ee_drive { None } else { wh.enc_key.clone() };
        let enabled = cfg.drive_enabled(&wh.drive_id);
        if dk.is_none() && legacy_key.is_none() {
            // E2EE drive whose key we don't hold yet (storage never unlocked
            // in the app), or a legacy drive never opened on the web: listed,
            // but nothing can be read or written safely.
            drive_statuses.push(DriveStatus {
                id: wh.drive_id.clone(),
                name: wh.name.clone(),
                enabled,
                file_count: 0,
                locked: e2ee_drive,
            });
            continue;
        }
        new_index.drives.insert(
            wh.drive_id.clone(),
            DriveSecrets {
                name: wh.name.clone(),
                webhook_url: wh.webhook_url.clone(),
                legacy_key,
                dk,
            },
        );
        let file_count = if enabled {
            match reconcile_drive(
                ctx,
                &api,
                &wh.drive_id,
                &wh.name,
                icon.as_deref(),
                &mut new_index,
            )
            .await
            {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("sync: drive {} : {e}", wh.drive_id);
                    0
                }
            }
        } else {
            0
        };
        drive_statuses.push(DriveStatus {
            id: wh.drive_id.clone(),
            name: wh.name.clone(),
            enabled,
            file_count,
            locked: false,
        });
    }

    *ctx.index.write() = new_index;

    let mut s = ctx.status.write();
    s.drives = drive_statuses;
    s.state = "synced".into();
    s.last_error = None;
    s.last_sync_at = Some(now_ms());
    drop(s);
    ctx.touch_status();
}

/// Create a placeholder *directory* under `parent` if it isn't there yet, and
/// give it the Drivecord folder icon. Plain `std::fs::create_dir` inside a
/// connected Cloud Files sync root does not reliably stick — the folder has to
/// be a placeholder like the files.
fn ensure_dir_placeholder(
    parent: &std::path::Path,
    name: &str,
    icon: Option<&std::path::Path>,
) -> std::path::PathBuf {
    let abs = parent.join(name);
    if !abs.exists() {
        let ph = PlaceholderFile::new(name)
            .metadata(Metadata::directory())
            .mark_in_sync();
        if let Err(e) = ph.create::<&std::path::Path>(parent) {
            // Fall back to a plain dir (better than nothing outside a sync root).
            let _ = std::fs::create_dir_all(&abs);
            eprintln!("sync: création du dossier fantôme {abs:?} : {e}");
        }
    }
    if let Some(icon) = icon {
        super::folder_style::apply_icon(&abs, icon);
    }
    abs
}

/// Ensure `<root>/<drive name>/…` mirrors one drive's tree. Returns the file count.
async fn reconcile_drive(
    ctx: &EngineCtx,
    api: &api::Api,
    drive_id: &str,
    drive_name: &str,
    icon: Option<&std::path::Path>,
    index: &mut FileIndex,
) -> Result<u32, String> {
    // Every folder goes into the LIVE index before it's created on disk: the
    // fs watcher sees the new directory too, and must not mistake it for one
    // the user dropped in (that would create a duplicate folder cloud-side).
    let expect_folder = |abs: PathBuf, folder_id: &str| {
        ctx.index
            .write()
            .path_to_folder
            .insert(abs, (drive_id.to_string(), folder_id.to_string()));
    };
    expect_folder(ctx.root.join(sanitize(drive_name)), "");
    let drive_root = ensure_dir_placeholder(&ctx.root, &sanitize(drive_name), icon);
    index
        .path_to_folder
        .insert(drive_root.clone(), (drive_id.to_string(), String::new()));

    let mut tree = api.get_tree(drive_id, None).await?;
    let dk = index.drives.get(drive_id).and_then(|d| d.dk);

    // End-to-end encrypted drive: open folder names, and each file's key +
    // metadata (real name, plain size) — the server only has ciphertext.
    // Anything that doesn't decrypt is left out rather than shown garbled.
    if let Some(dk) = dk {
        tree.folders.retain_mut(|f| match f.enc_name.as_deref() {
            Some(enc) => match e2ee::open_folder_name(&dk, &f.id, enc) {
                Ok(name) => {
                    f.name = name;
                    true
                }
                Err(e) => {
                    eprintln!("sync: dossier {} illisible : {e}", f.id);
                    false
                }
            },
            None => true,
        });
        tree.files.retain_mut(|file| {
            if !file.is_v1() {
                return true;
            }
            let opened = (|| {
                let fk = e2ee::unwrap_file_key(
                    &dk,
                    &file.id,
                    file.fk_wrapped.as_deref().ok_or("clé de fichier absente")?,
                )?;
                let meta = e2ee::open_meta(
                    &fk,
                    &file.id,
                    file.enc_meta.as_deref().ok_or("métadonnées absentes")?,
                )?;
                Ok::<_, String>((fk, meta))
            })();
            match opened {
                Ok((fk, meta)) => {
                    file.fk = Some(fk);
                    file.meta = Some(meta);
                    true
                }
                Err(e) => {
                    eprintln!("sync: fichier {} illisible : {e}", file.id);
                    false
                }
            }
        });
    } else {
        // Legacy drive: a v1 file can't exist there, but never hand one out undecrypted.
        tree.files.retain(|f| !f.is_v1());
    }

    // Folders: repeatedly place any folder whose parent we've already
    // resolved, until nothing more progresses (handles arbitrary nesting
    // without assuming the API returns parents before children).
    let mut resolved: HashMap<String, PathBuf> = HashMap::new();
    resolved.insert(String::new(), drive_root.clone());
    let mut remaining: Vec<&api::FolderEntry> = tree.folders.iter().collect();
    loop {
        let mut progressed = false;
        remaining.retain(|f| {
            let Some(parent_abs) = resolved.get(&f.parent_id).cloned() else {
                return true;
            };
            expect_folder(parent_abs.join(sanitize(&f.name)), &f.id);
            let abs = ensure_dir_placeholder(&parent_abs, &sanitize(&f.name), icon);
            index
                .path_to_folder
                .insert(abs.clone(), (drive_id.to_string(), f.id.clone()));
            resolved.insert(f.id.clone(), abs);
            progressed = true;
            false
        });
        if !progressed || remaining.is_empty() {
            break;
        }
    }

    let mut count = 0u32;
    for file in &tree.files {
        let Some(parent_abs) = resolved.get(&file.parent_id) else {
            continue; // orphaned (parent folder missing/trashed) — skip this pass
        };
        let name = sanitize(file.display_name());
        let abs = parent_abs.join(&name);

        if !abs.exists() {
            // Known before it exists (see `expect_folder`): never "new local content".
            ctx.index
                .write()
                .path_to_file
                .insert(abs.clone(), (drive_id.to_string(), file.id.clone()));
            let blob = format!("{drive_id}\u{0}{}", file.id).into_bytes();
            // `DriveFile.size` is the size of what actually got uploaded
            // (ciphertext). The placeholder's declared size must match what
            // `fetch_data` will actually write (the *decrypted* bytes), or
            // Windows treats the hydration as incomplete.
            let mut meta = Metadata::file().size(file.logical_size());
            if let Some(t) = filetime_from_unix_ms(file.updated_at) {
                meta = meta.written(t);
            }
            let placeholder = PlaceholderFile::new(&name)
                .metadata(meta)
                .mark_in_sync()
                .blob(blob);
            if let Err(e) = placeholder.create::<&std::path::Path>(parent_abs) {
                eprintln!("sync: création du fantôme {abs:?} : {e}");
                continue;
            }
        }

        index
            .files
            .insert(file.id.clone(), (drive_id.to_string(), file.clone()));
        index
            .path_to_file
            .insert(abs, (drive_id.to_string(), file.id.clone()));
        count += 1;
    }

    Ok(count)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_reserved_chars() {
        assert_eq!(sanitize("a/b:c*d"), "a_b_c_d");
        assert_eq!(sanitize("trailing.dot. "), "trailing.dot");
        assert_eq!(sanitize(""), "_");
    }
}
