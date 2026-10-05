//! Disk -> cloud direction: a real file dropped into a drive's subfolder gets
//! encrypted, chunked, uploaded to that drive's Discord webhook, and recorded
//! via the same `POST /api/drive/[id]/files` the web client uses. The local
//! file is then converted into an in-sync placeholder so it isn't re-uploaded.

use std::path::Path;

use cloud_filter::placeholder::{ConvertOptions, Placeholder};

use super::{api, crypto, discord, e2ee, DriveSecrets, EngineCtx};

fn guess_mime(filename: &str) -> String {
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "json" => "application/json",
        "zip" => "application/zip",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "mov" => "video/quicktime",
        _ => "",
    }
    .to_string()
}

/// Encrypt + upload `abs_path` as a new file `filename` under `parent_id`,
/// then collapse it into an in-sync placeholder. `on_progress(frac)` is called
/// with 0.0..1.0 as chunks go up.
pub async fn upload_new_file<F: Fn(f32)>(
    ctx: &EngineCtx,
    drive_id: &str,
    secrets: &DriveSecrets,
    parent_id: &str,
    abs_path: &Path,
    filename: &str,
    on_progress: F,
) -> Result<String, String> {
    let bytes = tokio::fs::read(abs_path).await.map_err(|e| e.to_string())?;
    let mime_type = guess_mime(filename);

    let id = if let Some(dk) = secrets.dk {
        let file = LocalFile {
            path: abs_path,
            name: filename,
            mime: &mime_type,
            bytes,
        };
        upload_v1(ctx, drive_id, secrets, &dk, parent_id, file, on_progress).await?
    } else {
        upload_legacy(
            ctx,
            drive_id,
            secrets,
            parent_id,
            filename,
            &mime_type,
            &bytes,
            on_progress,
        )
        .await?
    };

    // Collapse the local real file into a placeholder so the watcher / next
    // reconcile pass doesn't see it as "new" again.
    let blob = format!("{drive_id}\u{0}{id}").into_bytes();
    if let Ok(file) = std::fs::File::open(abs_path) {
        let mut placeholder: Placeholder = file.into();
        if let Err(e) = placeholder
            .convert_to_placeholder(ConvertOptions::default().mark_in_sync().blob(blob), None)
        {
            eprintln!("sync: conversion en fantôme après upload ({abs_path:?}) : {e}");
        }
    }

    Ok(id)
}

struct LocalFile<'a> {
    path: &'a Path,
    name: &'a str,
    mime: &'a str,
    bytes: Vec<u8>,
}

/// End-to-end encrypted drive (format v1, same as the web client): fresh file
/// key + nonce prefix, 8 MiB chunks each sealed separately, opaque Discord
/// attachment names, and only sealed metadata reaches the server.
async fn upload_v1<F: Fn(f32)>(
    ctx: &EngineCtx,
    drive_id: &str,
    secrets: &DriveSecrets,
    dk: &e2ee::RawKey,
    parent_id: &str,
    file: LocalFile<'_>,
    on_progress: F,
) -> Result<String, String> {
    let id = nanoid::nanoid!(21);
    let fk = e2ee::random_key();
    let prefix = e2ee::random_prefix();
    let cipher_chunks = e2ee::encrypt_file(&fk, &prefix, &id, &file.bytes)?;
    let total = cipher_chunks.len();

    let mut chunks = Vec::with_capacity(total);
    for (index, ct) in cipher_chunks.iter().enumerate() {
        let mut chunk = discord::upload_chunk(
            &ctx.http,
            &secrets.webhook_url,
            ct,
            &format!("{id}.part{index}"),
        )
        .await?;
        chunk.index = index as u32;
        chunks.push(chunk);
        on_progress((chunks.len() as f32 / total as f32) * 0.97);
    }

    let mtime = std::fs::metadata(file.path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);
    let meta = e2ee::FileMeta {
        name: file.name.to_string(),
        mime: if file.mime.is_empty() {
            "application/octet-stream".to_string()
        } else {
            file.mime.to_string()
        },
        size: file.bytes.len() as u64,
        mtime,
        sha256: None,
    };
    let fk_wrapped = e2ee::wrap_file_key(dk, &id, &fk)?;
    let enc_meta = e2ee::seal_meta(&fk, &id, &meta)?;
    let nonce_prefix = e2ee::b64(&prefix);
    let cipher_size: u64 = chunks.iter().map(|c| c.size).sum();

    api::Api::new(ctx.http.clone(), ctx.token.clone())
        .create_file(
            drive_id,
            &id,
            parent_id,
            "",
            cipher_size,
            "application/octet-stream",
            e2ee::CHUNK_CIPHER,
            &chunks,
            None,
            Some(api::V1Fields {
                fk_wrapped: &fk_wrapped,
                enc_meta: &enc_meta,
                nonce_prefix: &nonce_prefix,
            }),
        )
        .await?;
    Ok(id)
}

/// Legacy drive (server-held key): one AES-GCM blob, chunked for Discord.
#[allow(clippy::too_many_arguments)]
async fn upload_legacy<F: Fn(f32)>(
    ctx: &EngineCtx,
    drive_id: &str,
    secrets: &DriveSecrets,
    parent_id: &str,
    filename: &str,
    mime_type: &str,
    bytes: &[u8],
    on_progress: F,
) -> Result<String, String> {
    let key = secrets
        .legacy_key
        .as_deref()
        .ok_or("clé du drive absente")?;
    let (enc_iv, ciphertext) = crypto::encrypt(key, bytes)?;

    let plan = discord::plan_chunks(ciphertext.len() as u64);
    let total = plan.len().max(1);
    let mut chunks = Vec::with_capacity(total);
    for (index, start, end) in &plan {
        let part_name = if plan.len() > 1 {
            format!("{filename}.part{index}")
        } else {
            filename.to_string()
        };
        let mut chunk = discord::upload_chunk(
            &ctx.http,
            &secrets.webhook_url,
            &ciphertext[*start as usize..*end as usize],
            &part_name,
        )
        .await?;
        chunk.index = *index;
        chunks.push(chunk);
        on_progress((chunks.len() as f32 / total as f32) * 0.97);
    }

    let id = nanoid::nanoid!(12);
    api::Api::new(ctx.http.clone(), ctx.token.clone())
        .create_file(
            drive_id,
            &id,
            parent_id,
            filename,
            bytes.len() as u64,
            mime_type,
            discord::CHUNK_SIZE,
            &chunks,
            Some(enc_iv.as_str()),
            None,
        )
        .await?;
    Ok(id)
}
