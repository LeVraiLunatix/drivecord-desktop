//! Wire types for Drivecord's `/api/v1`. Field names match the JSON exactly
//! (camelCase), so keep `rename_all` in sync with the server.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeResponse {
    pub drive: String,
    pub drive_id: String,
    pub scopes: Vec<String>,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub id: String,
    pub drive_id: String,
    pub parent_id: String,
    pub filename: String,
    pub size: u64,
    pub mime_type: String,
    pub chunk_size: u64,
    #[serde(default)]
    pub chunks: Vec<ChunkRef>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub trashed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderEntry {
    pub id: String,
    pub drive_id: String,
    pub parent_id: String,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub trashed: bool,
    #[serde(default)]
    pub color: Option<String>,
}

/// `GET /api/v1/files` in sync mode.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesPage {
    #[serde(default)]
    pub files: Vec<FileEntry>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FilesResponse {
    #[serde(default)]
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FoldersResponse {
    #[serde(default)]
    pub folders: Vec<FolderEntry>,
}
