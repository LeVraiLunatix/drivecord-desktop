//! Disk -> cloud direction: a real file dropped into a drive's subfolder gets
//! encrypted, chunked, uploaded to that drive's Discord webhook, and recorded
//! via the same `POST /api/drive/[id]/files` the web client uses. The local
//! file is then converted into an in-sync placeholder so it isn't re-uploaded.

use std::path::Path;

use cloud_filter::placeholder::{ConvertOptions, Placeholder};

use super::{api, crypto, discord, DriveSecrets, EngineCtx};

fn guess_mime(filename: &str) -> String {
    let ext = filename.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
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
/// then collapse it into an in-sync placeholder.
pub async fn upload_new_file(
    ctx: &EngineCtx,
    drive_id: &str,
    secrets: &DriveSecrets,
    parent_id: &str,
    abs_path: &Path,
    filename: &str,
) -> Result<String, String> {
    let bytes = tokio::fs::read(abs_path).await.map_err(|e| e.to_string())?;
    let mime_type = guess_mime(filename);

    let (enc_iv, ciphertext) = crypto::encrypt(&secrets.enc_key, &bytes)?;

    let plan = discord::plan_chunks(ciphertext.len() as u64);
    let mut chunks = Vec::with_capacity(plan.len().max(1));
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
    }

    let id = nanoid::nanoid!(12);
    api::Api::new(ctx.http.clone(), ctx.token.clone())
        .create_file(
            drive_id,
            &id,
            parent_id,
            filename,
            bytes.len() as u64,
            &mime_type,
            discord::CHUNK_SIZE,
            &chunks,
            Some(enc_iv.as_str()),
        )
        .await?;

    // Collapse the local real file into a placeholder so the watcher / next
    // reconcile pass doesn't see it as "new" again.
    let blob = format!("{drive_id}\u{0}{id}").into_bytes();
    if let Ok(file) = std::fs::File::open(abs_path) {
        let mut placeholder: Placeholder = file.into();
        if let Err(e) =
            placeholder.convert_to_placeholder(ConvertOptions::default().mark_in_sync().blob(blob), None)
        {
            eprintln!("sync: conversion en fantôme après upload ({abs_path:?}) : {e}");
        }
    }

    Ok(id)
}
