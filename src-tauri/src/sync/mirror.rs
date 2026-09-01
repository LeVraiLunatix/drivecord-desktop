//! Cloud -> disk reconciliation: fetch every drive's tree and make sure every
//! folder exists as a real directory and every file exists as an in-sync
//! placeholder. Runs on the sync worker thread, on an interval and on demand.

use std::{collections::HashMap, path::PathBuf};

use cloud_filter::{metadata::Metadata, placeholder_file::PlaceholderFile, utility::FileTime};

use super::{api, DriveSecrets, DriveStatus, EngineCtx, FileIndex};

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

    for wh in &webhooks {
        let Some(enc_key) = wh.enc_key.clone() else {
            // No drive key yet (never opened on the web) — nothing to sync safely.
            continue;
        };
        let enabled = cfg.drive_enabled(&wh.drive_id);
        new_index.drives.insert(
            wh.drive_id.clone(),
            DriveSecrets {
                name: wh.name.clone(),
                webhook_url: wh.webhook_url.clone(),
                enc_key,
            },
        );
        let file_count = if enabled {
            match reconcile_drive(ctx, &api, &wh.drive_id, &wh.name, &mut new_index).await {
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

/// Ensure `<root>/<drive name>/…` mirrors one drive's tree. Returns the file count.
async fn reconcile_drive(
    ctx: &EngineCtx,
    api: &api::Api,
    drive_id: &str,
    drive_name: &str,
    index: &mut FileIndex,
) -> Result<u32, String> {
    let drive_root = ctx.root.join(sanitize(drive_name));
    std::fs::create_dir_all(&drive_root).map_err(|e| e.to_string())?;
    index
        .path_to_folder
        .insert(drive_root.clone(), (drive_id.to_string(), String::new()));

    let tree = api.get_tree(drive_id, None).await?;

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
            let abs = parent_abs.join(sanitize(&f.name));
            let _ = std::fs::create_dir_all(&abs);
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
        let name = sanitize(&file.filename);
        let abs = parent_abs.join(&name);

        if !abs.exists() {
            let blob = format!("{drive_id}\u{0}{}", file.id).into_bytes();
            // `DriveFile.size` is the size of what actually got uploaded — for
            // E2EE files that's ciphertext (plaintext + 16-byte AES-GCM tag).
            // The placeholder's declared size must match what `fetch_data`
            // will actually write (the *decrypted* bytes), or Windows treats
            // the hydration as incomplete.
            let logical_size = if file.enc_iv.is_some() {
                file.size.saturating_sub(16)
            } else {
                file.size
            };
            let mut meta = Metadata::file().size(logical_size);
            if let Some(t) = filetime_from_unix_ms(file.updated_at) {
                meta = meta.written(t);
            }
            let placeholder = PlaceholderFile::new(&name).metadata(meta).mark_in_sync().blob(blob);
            if let Err(e) = placeholder.create::<&std::path::Path>(parent_abs) {
                eprintln!("sync: création du fantôme {abs:?} : {e}");
                continue;
            }
        }

        index.files.insert(file.id.clone(), (drive_id.to_string(), file.clone()));
        index.path_to_file.insert(abs, (drive_id.to_string(), file.id.clone()));
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
