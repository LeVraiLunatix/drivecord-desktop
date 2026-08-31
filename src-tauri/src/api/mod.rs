//! HTTP client for Drivecord's public API (`/api/v1`).
//!
//! One `ApiClient` is bound to a server URL + API key. `send` injects the
//! bearer token and transparently backs off on `429` using the `Retry-After`
//! header. File bytes always transit the Drivecord server (a key holder has no
//! Discord webhook URL and no E2EE key), so uploads/downloads are plaintext.

pub mod models;

use std::time::Duration;

use reqwest::{header, multipart, Client, Response, StatusCode};
use serde::Deserialize;
use serde_json::json;

use crate::error::{AppError, AppResult};
use models::*;

const MAX_RETRIES: u32 = 4;
const DEFAULT_RETRY_AFTER: u64 = 2;
const MAX_RETRY_AFTER: u64 = 60;
const USER_AGENT: &str = concat!("drivecord-desktop/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicLink {
    pub token: String,
    pub url: String,
}

#[derive(Clone)]
pub struct ApiClient {
    http: Client,
    base: String,
    key: String,
}

impl ApiClient {
    pub fn new(server_url: &str, api_key: &str) -> AppResult<Self> {
        let http = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(120))
            .build()?;
        Ok(Self {
            http,
            base: server_url.trim_end_matches('/').to_string(),
            key: api_key.to_string(),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v1{}", self.base, path)
    }

    /// Send with bearer auth + `429` backoff. `make` is re-invoked per attempt
    /// because a `RequestBuilder` is single-use.
    async fn send<F>(&self, make: F) -> AppResult<Response>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let mut attempt = 0u32;
        loop {
            let resp = make().bearer_auth(&self.key).send().await?;
            if resp.status() == StatusCode::TOO_MANY_REQUESTS && attempt < MAX_RETRIES {
                let wait = retry_after(&resp);
                attempt += 1;
                tokio::time::sleep(Duration::from_secs(wait)).await;
                continue;
            }
            return Ok(resp);
        }
    }

    async fn json<T: serde::de::DeserializeOwned>(resp: Response) -> AppResult<T> {
        if resp.status().is_success() {
            return resp.json::<T>().await.map_err(AppError::from);
        }
        Err(classify(resp).await)
    }

    async fn no_content(resp: Response) -> AppResult<()> {
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(classify(resp).await)
        }
    }

    // ── metadata ───────────────────────────────────────────────────────────

    pub async fn me(&self) -> AppResult<MeResponse> {
        let r = self.send(|| self.http.get(self.url("/me"))).await?;
        Self::json(r).await
    }

    pub async fn list_files(&self, parent_id: &str, limit: u32) -> AppResult<Vec<FileEntry>> {
        let limit = limit.to_string();
        let r = self
            .send(|| {
                self.http
                    .get(self.url("/files"))
                    .query(&[("parentId", parent_id), ("limit", limit.as_str())])
            })
            .await?;
        Ok(Self::json::<FilesResponse>(r).await?.files)
    }

    /// Sync-mode listing (`recursive`, `updatedSince` ms, `cursor`).
    pub async fn list_files_sync(
        &self,
        recursive: bool,
        updated_since: Option<i64>,
        cursor: Option<&str>,
    ) -> AppResult<FilesPage> {
        let r = self
            .send(|| {
                let mut q: Vec<(&str, String)> = Vec::new();
                if recursive {
                    q.push(("recursive", "1".to_string()));
                }
                if let Some(s) = updated_since {
                    q.push(("updatedSince", s.to_string()));
                }
                if let Some(c) = cursor {
                    q.push(("cursor", c.to_string()));
                }
                self.http.get(self.url("/files")).query(&q)
            })
            .await?;
        Self::json(r).await
    }

    pub async fn list_folders(&self, recursive: bool) -> AppResult<Vec<FolderEntry>> {
        let r = self
            .send(|| {
                let req = self.http.get(self.url("/folders"));
                if recursive {
                    req.query(&[("recursive", "1")])
                } else {
                    req
                }
            })
            .await?;
        Ok(Self::json::<FoldersResponse>(r).await?.folders)
    }

    pub async fn create_folder(&self, name: &str, parent_id: &str) -> AppResult<FolderEntry> {
        let body = json!({ "name": name, "parentId": parent_id });
        let r = self
            .send(|| self.http.post(self.url("/folders")).json(&body))
            .await?;
        Self::json(r).await
    }

    pub async fn delete_folder(&self, id: &str) -> AppResult<()> {
        let r = self
            .send(|| self.http.delete(self.url(&format!("/folders/{id}"))))
            .await?;
        Self::no_content(r).await
    }

    pub async fn delete_file(&self, id: &str) -> AppResult<()> {
        let r = self
            .send(|| self.http.delete(self.url(&format!("/files/{id}"))))
            .await?;
        Self::no_content(r).await
    }

    // ── transfers ──────────────────────────────────────────────────────────

    /// Whole-file upload in one request. Bound by the server's body limit
    /// (~45 MiB) — the caller picks this vs. chunked based on size.
    pub async fn upload_simple(
        &self,
        bytes: Vec<u8>,
        filename: &str,
        mime: &str,
        parent_id: &str,
    ) -> AppResult<FileEntry> {
        let part = multipart::Part::bytes(bytes)
            .file_name(filename.to_string())
            .mime_str(mime)
            .map_err(AppError::from)?;
        let form = multipart::Form::new()
            .part("file", part)
            .text("parentId", parent_id.to_string())
            .text("filename", filename.to_string());
        let resp = self
            .http
            .post(self.url("/files"))
            .bearer_auth(&self.key)
            .multipart(form)
            .send()
            .await?;
        Self::json(resp).await
    }

    /// Upload one ~9.5 MiB slice; returns the `ChunkRef` to collect for
    /// `finalize_chunked`. No auto-retry (the sync queue re-drives failures).
    pub async fn upload_chunk(&self, index: u32, bytes: Vec<u8>) -> AppResult<ChunkRef> {
        let part = multipart::Part::bytes(bytes).file_name(format!("part{index}"));
        let form = multipart::Form::new()
            .part("chunk", part)
            .text("index", index.to_string());
        let resp = self
            .http
            .post(self.url("/files/chunks"))
            .bearer_auth(&self.key)
            .multipart(form)
            .send()
            .await?;
        Self::json(resp).await
    }

    pub async fn finalize_chunked(
        &self,
        filename: &str,
        mime: &str,
        chunk_size: u64,
        parent_id: &str,
        chunks: &[ChunkRef],
    ) -> AppResult<FileEntry> {
        let body = json!({
            "filename": filename,
            "mimeType": mime,
            "chunkSize": chunk_size,
            "parentId": parent_id,
            "chunks": chunks,
        });
        let r = self
            .send(|| self.http.post(self.url("/files")).json(&body))
            .await?;
        Self::json(r).await
    }

    /// Raw file bytes (server refreshes CDN URLs + decrypts as needed).
    pub async fn download(&self, id: &str) -> AppResult<Vec<u8>> {
        let resp = self
            .send(|| self.http.get(self.url(&format!("/files/{id}/download"))))
            .await?;
        if !resp.status().is_success() {
            return Err(classify(resp).await);
        }
        Ok(resp.bytes().await?.to_vec())
    }

    pub async fn create_public_link(&self, id: &str) -> AppResult<PublicLink> {
        let r = self
            .send(|| self.http.post(self.url(&format!("/files/{id}/public"))))
            .await?;
        Self::json(r).await
    }

    pub async fn delete_public_link(&self, id: &str) -> AppResult<()> {
        let r = self
            .send(|| self.http.delete(self.url(&format!("/files/{id}/public"))))
            .await?;
        Self::no_content(r).await
    }
}

fn retry_after(resp: &Response) -> u64 {
    resp.headers()
        .get(header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_RETRY_AFTER)
        .clamp(1, MAX_RETRY_AFTER)
}

async fn classify(resp: Response) -> AppError {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    match status {
        StatusCode::UNAUTHORIZED => AppError::Unauthorized,
        StatusCode::FORBIDDEN => AppError::MissingScope("read/write"),
        StatusCode::NOT_FOUND => AppError::NotFound,
        StatusCode::TOO_MANY_REQUESTS => AppError::RateLimited,
        _ => AppError::Unexpected {
            status: status.as_u16(),
            body,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn me_ok() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .and(header("authorization", "Bearer dvc_test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "drive": "Mon site",
                "driveId": "abc",
                "scopes": ["read", "write"],
            })))
            .mount(&server)
            .await;

        let client = ApiClient::new(&server.uri(), "dvc_test").unwrap();
        let me = client.me().await.unwrap();
        assert_eq!(me.drive, "Mon site");
        assert_eq!(me.drive_id, "abc");
        assert_eq!(me.scopes, vec!["read".to_string(), "write".to_string()]);
    }

    #[tokio::test]
    async fn retries_once_on_429_then_succeeds() {
        let server = MockServer::start().await;
        // Fallback (registered first → lowest priority): the eventual 200.
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "drive": "d", "driveId": "i", "scopes": [],
            })))
            .mount(&server)
            .await;
        // Registered last → matched first, but only once.
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "1"))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        let client = ApiClient::new(&server.uri(), "dvc_x").unwrap();
        assert!(client.me().await.is_ok());
    }

    #[tokio::test]
    async fn maps_401_to_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({ "error": "nope" })))
            .mount(&server)
            .await;

        let client = ApiClient::new(&server.uri(), "dvc_x").unwrap();
        assert!(matches!(client.me().await, Err(AppError::Unauthorized)));
    }

    #[tokio::test]
    async fn sync_listing_reads_next_cursor() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "files": [],
                "nextCursor": "1717000000000_abc123",
            })))
            .mount(&server)
            .await;

        let client = ApiClient::new(&server.uri(), "dvc_x").unwrap();
        let page = client.list_files_sync(true, None, None).await.unwrap();
        assert_eq!(page.next_cursor.as_deref(), Some("1717000000000_abc123"));
    }
}
