//! Native port of the browser's Discord webhook client
//! (`src/lib/discord/client.ts` + `chunking.ts` + `constants.ts`).
//!
//! Same wire behaviour: chunks uploaded as individual `?wait=true` webhook
//! messages, CDN URLs refreshed by re-fetching the parent message, downloads
//! hit `cdn.discordapp.com` directly (no CORS proxy needed off the browser).

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// 9.5 MiB — just under Discord's 10 MiB free-tier attachment ceiling.
pub const CHUNK_SIZE: u64 = 9_961_472;

const RETRY_MAX_ATTEMPTS: u32 = 12;
const RETRY_BASE_DELAY_MS: u64 = 500;
const RETRY_MAX_DELAY_MS: u64 = 30_000;

pub type Result<T> = std::result::Result<T, String>;

/// One Discord message holding one chunk. Field names match the JSON the web
/// client persists via `POST /api/drive/[id]/files`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkRef {
    pub index: u32,
    pub size: u64,
    pub message_id: String,
    pub attachment_id: String,
    pub url: String,
    #[serde(default)]
    pub expires_at: i64,
}

/// `[start, end)` byte ranges, last one possibly short. Mirrors `planChunks`.
pub fn plan_chunks(total: u64) -> Vec<(u32, u64, u64)> {
    if total == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0u64;
    let mut i = 0u32;
    while start < total {
        let end = (start + CHUNK_SIZE).min(total);
        out.push((i, start, end));
        start = end;
        i += 1;
    }
    out
}

/// Unix-ms expiry from a Discord CDN URL's hex `ex` query param (0 if absent).
pub fn parse_cdn_expiry(url: &str) -> i64 {
    let Some(q) = url.split('?').nth(1) else { return 0 };
    for pair in q.split('&') {
        if let Some(hex) = pair.strip_prefix("ex=") {
            if let Ok(secs) = i64::from_str_radix(hex, 16) {
                return secs * 1000;
            }
        }
    }
    0
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

async fn with_retry<T, F, Fut>(what: &str, mut f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut attempt = 0u32;
    loop {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                attempt += 1;
                if attempt >= RETRY_MAX_ATTEMPTS {
                    return Err(format!("{what}: abandon après {attempt} essais — {e}"));
                }
                let delay = (RETRY_BASE_DELAY_MS << attempt.min(6)).min(RETRY_MAX_DELAY_MS);
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
        }
    }
}

#[derive(Deserialize)]
struct Attachment {
    id: String,
    url: String,
}

#[derive(Deserialize)]
struct MessageResp {
    id: String,
    #[serde(default)]
    attachments: Vec<Attachment>,
}

/// Upload one chunk as a single webhook message.
pub async fn upload_chunk(
    http: &reqwest::Client,
    webhook_url: &str,
    bytes: &[u8],
    filename: &str,
) -> Result<ChunkRef> {
    with_retry("upload_chunk", || async {
        let part = reqwest::multipart::Part::bytes(bytes.to_vec())
            .file_name(filename.to_string())
            .mime_str("application/octet-stream")
            .map_err(|e| e.to_string())?;
        let form = reqwest::multipart::Form::new().part("files[0]", part);
        let res = http
            .post(format!("{webhook_url}?wait=true"))
            .multipart(form)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            return Err(format!("HTTP {}", res.status()));
        }
        let msg: MessageResp = res.json().await.map_err(|e| e.to_string())?;
        let att = msg
            .attachments
            .into_iter()
            .next()
            .ok_or_else(|| "message sans pièce jointe".to_string())?;
        Ok(ChunkRef {
            index: 0,
            size: bytes.len() as u64,
            message_id: msg.id,
            attachment_id: att.id,
            expires_at: parse_cdn_expiry(&att.url),
            url: att.url,
        })
    })
    .await
}

/// Re-fetch a message to get a fresh signed CDN URL for one of its attachments.
pub async fn refresh_chunk_url(
    http: &reqwest::Client,
    webhook_url: &str,
    message_id: &str,
    attachment_id: &str,
) -> Result<(String, i64)> {
    with_retry("refresh_chunk_url", || async {
        let res = http
            .get(format!("{webhook_url}/messages/{message_id}"))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            return Err(format!("HTTP {}", res.status()));
        }
        let msg: MessageResp = res.json().await.map_err(|e| e.to_string())?;
        let att = msg
            .attachments
            .into_iter()
            .find(|a| a.id == attachment_id)
            .ok_or_else(|| "pièce jointe introuvable".to_string())?;
        let ex = parse_cdn_expiry(&att.url);
        Ok((att.url, ex))
    })
    .await
}

/// Download one chunk's bytes, refreshing the CDN URL first if it's near expiry
/// or if the CDN rejects it.
pub async fn download_chunk(
    http: &reqwest::Client,
    webhook_url: &str,
    chunk: &ChunkRef,
) -> Result<Vec<u8>> {
    let mut url = chunk.url.clone();
    if chunk.expires_at > 0 && chunk.expires_at - now_ms() < 30_000 {
        if let Ok((fresh, _)) =
            refresh_chunk_url(http, webhook_url, &chunk.message_id, &chunk.attachment_id).await
        {
            url = fresh;
        }
    }
    with_retry("download_chunk", || {
        let url = url.clone();
        async move {
            let res = http.get(&url).send().await.map_err(|e| e.to_string())?;
            if res.status() == 403 || res.status() == 404 {
                let (fresh, _) = refresh_chunk_url(
                    http,
                    webhook_url,
                    &chunk.message_id,
                    &chunk.attachment_id,
                )
                .await?;
                let res2 = http.get(&fresh).send().await.map_err(|e| e.to_string())?;
                if !res2.status().is_success() {
                    return Err(format!("HTTP {}", res2.status()));
                }
                return Ok(res2.bytes().await.map_err(|e| e.to_string())?.to_vec());
            }
            if !res.status().is_success() {
                return Err(format!("HTTP {}", res.status()));
            }
            Ok(res.bytes().await.map_err(|e| e.to_string())?.to_vec())
        }
    })
    .await
}

/// Download every chunk in order and concatenate — the raw ciphertext blob.
pub async fn download_all(
    http: &reqwest::Client,
    webhook_url: &str,
    chunks: &[ChunkRef],
) -> Result<Vec<u8>> {
    let mut ordered = chunks.to_vec();
    ordered.sort_by_key(|c| c.index);
    let mut out = Vec::new();
    for c in &ordered {
        out.extend_from_slice(&download_chunk(http, webhook_url, c).await?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_chunks_matches_ts() {
        assert!(plan_chunks(0).is_empty());
        assert_eq!(plan_chunks(10), vec![(0, 0, 10)]);
        let p = plan_chunks(CHUNK_SIZE + 1);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0], (0, 0, CHUNK_SIZE));
        assert_eq!(p[1], (1, CHUNK_SIZE, CHUNK_SIZE + 1));
    }

    #[test]
    fn cdn_expiry() {
        assert_eq!(parse_cdn_expiry("https://cdn.discordapp.com/x?ex=66b1a0e0&is=1&hm=2"), 0x66b1a0e0 * 1000);
        assert_eq!(parse_cdn_expiry("https://cdn.discordapp.com/x"), 0);
    }
}
