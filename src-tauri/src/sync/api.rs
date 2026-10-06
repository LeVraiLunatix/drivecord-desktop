//! Thin client for the `drivecord.app` REST API used by the sync engine.
//!
//! Every route here already exists for the web client (`discloud/src/app/api/
//! drive/[driveId]/*` and `/api/webhooks`) — the sync engine authenticates the
//! same way the embedded shell does: the bearer JWT from the Credential
//! Manager (`crate::token::read`), see `discloud/src/proxy.ts`.

use serde::{Deserialize, Serialize};

use crate::sync::discord::ChunkRef;
use crate::sync::e2ee::{FileMeta, RawKey};

const BASE: &str = "https://drivecord.app";

pub type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Deserialize)]
pub struct Webhook {
    #[serde(rename = "driveId")]
    pub drive_id: String,
    #[serde(rename = "webhookUrl")]
    pub webhook_url: String,
    pub name: String,
    /// Legacy drives only (`e2eeVersion` 0): the server-held raw key.
    #[serde(rename = "encKey")]
    pub enc_key: Option<String>,
    /// ≥ 1: end-to-end encrypted — the drive key is only known client-side
    /// (handed over by the unlocked shell, see `keystore.rs`).
    #[serde(default, rename = "e2eeVersion")]
    pub e2ee_version: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FolderEntry {
    pub id: String,
    #[serde(rename = "parentId")]
    pub parent_id: String,
    pub name: String,
    /// E2EE drives: the real name, sealed with the drive key (`name` is "").
    #[serde(default, rename = "encName")]
    pub enc_name: Option<String>,
}

// `mime_type` / `chunk_size` are part of the wire format but unused by the
// sync engine today (hydration only needs size/chunks/encIv) — kept for parity
// with the web client's FileEntry and future use.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct FileEntry {
    pub id: String,
    #[serde(rename = "parentId")]
    pub parent_id: String,
    pub filename: String,
    pub size: u64,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(rename = "chunkSize")]
    pub chunk_size: u64,
    pub chunks: Vec<ChunkRef>,
    #[serde(rename = "encIv")]
    pub enc_iv: Option<String>,
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
    /// Format v1 (end-to-end): per-file key wrapped by the drive key, sealed
    /// metadata (name/type/plain size), per-file nonce prefix.
    #[serde(default, rename = "cryptoVersion")]
    pub crypto_version: Option<u32>,
    #[serde(default, rename = "fkWrapped")]
    pub fk_wrapped: Option<String>,
    #[serde(default, rename = "encMeta")]
    pub enc_meta: Option<String>,
    #[serde(default, rename = "noncePrefix")]
    pub nonce_prefix: Option<String>,
    /// Resolved by the mirror pass for v1 files (never on the wire).
    #[serde(skip)]
    pub fk: Option<RawKey>,
    #[serde(skip)]
    pub meta: Option<FileMeta>,
}

impl FileEntry {
    pub fn is_v1(&self) -> bool {
        self.crypto_version.unwrap_or(0) >= 1
    }

    /// Name to show on disk (decrypted for v1 files).
    pub fn display_name(&self) -> &str {
        self.meta
            .as_ref()
            .map_or(self.filename.as_str(), |m| m.name.as_str())
    }

    /// Size of the decrypted content — what the placeholder must declare.
    pub fn logical_size(&self) -> u64 {
        if let Some(m) = &self.meta {
            return m.size;
        }
        // Legacy single-IV files: ciphertext = plaintext + one 16-byte tag.
        if self.enc_iv.is_some() {
            self.size.saturating_sub(16)
        } else {
            self.size
        }
    }
}

// `trashed*Ids` only populate in incremental (`?since=`) mode — Phase C.
#[allow(dead_code)]
#[derive(Debug, Default, Deserialize)]
pub struct Tree {
    pub folders: Vec<FolderEntry>,
    pub files: Vec<FileEntry>,
    #[serde(default, rename = "trashedFolderIds")]
    pub trashed_folder_ids: Vec<String>,
    #[serde(default, rename = "trashedFileIds")]
    pub trashed_file_ids: Vec<String>,
}

#[derive(Serialize)]
struct CreateFileBody<'a> {
    id: &'a str,
    #[serde(rename = "parentId")]
    parent_id: &'a str,
    filename: &'a str,
    size: u64,
    #[serde(rename = "mimeType")]
    mime_type: &'a str,
    #[serde(rename = "chunkSize")]
    chunk_size: u64,
    chunks: &'a [ChunkRef],
    #[serde(rename = "encIv", skip_serializing_if = "Option::is_none")]
    enc_iv: Option<&'a str>,
    #[serde(rename = "cryptoVersion")]
    crypto_version: u32,
    #[serde(rename = "fkWrapped", skip_serializing_if = "Option::is_none")]
    fk_wrapped: Option<&'a str>,
    #[serde(rename = "encMeta", skip_serializing_if = "Option::is_none")]
    enc_meta: Option<&'a str>,
    #[serde(rename = "noncePrefix", skip_serializing_if = "Option::is_none")]
    nonce_prefix: Option<&'a str>,
}

/// The v1 (end-to-end) part of a new file record.
pub struct V1Fields<'a> {
    pub fk_wrapped: &'a str,
    pub enc_meta: &'a str,
    pub nonce_prefix: &'a str,
}

#[derive(Serialize, Default)]
pub struct PatchFileBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(rename = "parentId", skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trashed: Option<bool>,
    /// Rename of a v1 file: re-sealed metadata (`filename` is refused).
    #[serde(rename = "encMeta", skip_serializing_if = "Option::is_none")]
    pub enc_meta: Option<String>,
}

#[derive(Serialize)]
struct CreateFolderBody<'a> {
    id: &'a str,
    #[serde(rename = "parentId")]
    parent_id: &'a str,
    name: &'a str,
    #[serde(rename = "encName", skip_serializing_if = "Option::is_none")]
    enc_name: Option<&'a str>,
}

pub struct Api {
    http: reqwest::Client,
    token: String,
}

impl Api {
    pub fn new(http: reqwest::Client, token: String) -> Self {
        Self { http, token }
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{BASE}{path}"))
            .header("Authorization", format!("Bearer {}", self.token))
    }

    async fn json<T: for<'de> Deserialize<'de>>(res: reqwest::Response) -> Result<T> {
        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(format!("HTTP {status}: {body}"));
        }
        res.json::<T>().await.map_err(|e| e.to_string())
    }

    pub async fn list_webhooks(&self) -> Result<Vec<Webhook>> {
        let res = self
            .req(reqwest::Method::GET, "/api/webhooks")
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Self::json(res).await
    }

    pub async fn get_tree(&self, drive_id: &str, since: Option<i64>) -> Result<Tree> {
        let path = match since {
            Some(s) => format!("/api/drive/{drive_id}/tree?since={s}"),
            None => format!("/api/drive/{drive_id}/tree"),
        };
        let res = self
            .req(reqwest::Method::GET, &path)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Self::json(res).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_file(
        &self,
        drive_id: &str,
        id: &str,
        parent_id: &str,
        filename: &str,
        size: u64,
        mime_type: &str,
        chunk_size: u64,
        chunks: &[ChunkRef],
        enc_iv: Option<&str>,
        v1: Option<V1Fields<'_>>,
    ) -> Result<FileEntry> {
        let body = CreateFileBody {
            id,
            parent_id,
            filename,
            size,
            mime_type,
            chunk_size,
            chunks,
            enc_iv,
            crypto_version: u32::from(v1.is_some()),
            fk_wrapped: v1.as_ref().map(|v| v.fk_wrapped),
            enc_meta: v1.as_ref().map(|v| v.enc_meta),
            nonce_prefix: v1.as_ref().map(|v| v.nonce_prefix),
        };
        let res = self
            .req(
                reqwest::Method::POST,
                &format!("/api/drive/{drive_id}/files"),
            )
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Self::json(res).await
    }

    pub async fn get_file(&self, drive_id: &str, id: &str) -> Result<FileEntry> {
        let res = self
            .req(
                reqwest::Method::GET,
                &format!("/api/drive/{drive_id}/files/{id}"),
            )
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Self::json(res).await
    }

    pub async fn patch_file(&self, drive_id: &str, id: &str, body: &PatchFileBody) -> Result<()> {
        let res = self
            .req(
                reqwest::Method::PATCH,
                &format!("/api/drive/{drive_id}/files/{id}"),
            )
            .json(body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            return Err(format!("HTTP {}", res.status()));
        }
        Ok(())
    }

    pub async fn create_folder(
        &self,
        drive_id: &str,
        id: &str,
        parent_id: &str,
        name: &str,
        enc_name: Option<&str>,
    ) -> Result<()> {
        let body = CreateFolderBody {
            id,
            parent_id,
            name,
            enc_name,
        };
        let res = self
            .req(
                reqwest::Method::POST,
                &format!("/api/drive/{drive_id}/folders"),
            )
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            return Err(format!("HTTP {}", res.status()));
        }
        Ok(())
    }
}
