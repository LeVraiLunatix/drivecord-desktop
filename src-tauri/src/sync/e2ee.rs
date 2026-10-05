//! End-to-end encryption, format v1 — byte-compatible with the web client's
//! `discloud/src/lib/crypto/e2ee/*` (wrap.ts, meta.ts, file-cipher.ts).
//!
//!  - Sealed blobs (wrapped keys, file metadata, folder names):
//!    `v1.<b64 iv(12)>.<b64 ct‖tag>`, AES-256-GCM, AAD = a context string.
//!  - Drive key (DK) → wraps each file key (FK), AAD `drivecord:fk:v1:<fileId>`.
//!  - FK → seals the file metadata JSON, AAD `drivecord:meta:v1:<fileId>`.
//!  - DK → seals folder names, AAD `drivecord:folder:v1:<folderId>`.
//!  - File content: 8 MiB plaintext chunks, each `+16` bytes of tag;
//!    IV(i) = noncePrefix(7) ‖ u32be(i) ‖ lastFlag, AAD `drivecord:chunk:v1:<fileId>:<i>`.

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng, Payload},
    AeadCore, Aes256Gcm, Key, Nonce,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};

pub type Result<T> = std::result::Result<T, String>;
pub type RawKey = [u8; 32];

pub const CHUNK_PLAIN: u64 = 8 * 1024 * 1024;
pub const TAG_LEN: u64 = 16;
pub const CHUNK_CIPHER: u64 = CHUNK_PLAIN + TAG_LEN;
pub const NONCE_PREFIX_LEN: usize = 7;

fn ctx_fk(file_id: &str) -> String {
    format!("drivecord:fk:v1:{file_id}")
}
fn ctx_meta(file_id: &str) -> String {
    format!("drivecord:meta:v1:{file_id}")
}
fn ctx_folder(folder_id: &str) -> String {
    format!("drivecord:folder:v1:{folder_id}")
}
fn ctx_chunk(file_id: &str, index: u32) -> String {
    format!("drivecord:chunk:v1:{file_id}:{index}")
}

fn cipher(key: &RawKey) -> Aes256Gcm {
    Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key))
}

pub fn random_key() -> RawKey {
    Aes256Gcm::generate_key(&mut OsRng).into()
}

pub fn key_from_b64(s: &str) -> Result<RawKey> {
    let raw = B64
        .decode(s.trim())
        .map_err(|e| format!("clé base64 invalide: {e}"))?;
    raw.try_into()
        .map_err(|_| "clé de drive : 32 octets attendus".to_string())
}

/// `v1.<iv>.<ct>` under `key`, bound to `context`.
pub fn seal(key: &RawKey, plain: &[u8], context: &str) -> Result<String> {
    let iv = Aes256Gcm::generate_nonce(&mut OsRng);
    let ct = cipher(key)
        .encrypt(
            &iv,
            Payload {
                msg: plain,
                aad: context.as_bytes(),
            },
        )
        .map_err(|_| "chiffrement AES-GCM échoué".to_string())?;
    Ok(format!("v1.{}.{}", B64.encode(iv), B64.encode(ct)))
}

pub fn open(key: &RawKey, blob: &str, context: &str) -> Result<Vec<u8>> {
    let mut parts = blob.split('.');
    let (Some("v1"), Some(iv), Some(ct), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err("format de blob chiffré inconnu".into());
    };
    let iv = B64.decode(iv).map_err(|e| e.to_string())?;
    let ct = B64.decode(ct).map_err(|e| e.to_string())?;
    if iv.len() != 12 {
        return Err("IV de 12 octets attendu".into());
    }
    cipher(key)
        .decrypt(
            Nonce::from_slice(&iv),
            Payload {
                msg: &ct,
                aad: context.as_bytes(),
            },
        )
        .map_err(|_| "déchiffrement impossible (mauvaise clé ou données altérées)".to_string())
}

/// Sealed file metadata (the JSON inside `encMeta`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMeta {
    pub name: String,
    pub mime: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtime: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

pub fn unwrap_file_key(dk: &RawKey, file_id: &str, fk_wrapped: &str) -> Result<RawKey> {
    open(dk, fk_wrapped, &ctx_fk(file_id))?
        .try_into()
        .map_err(|_| "clé de fichier : 32 octets attendus".to_string())
}

pub fn wrap_file_key(dk: &RawKey, file_id: &str, fk: &RawKey) -> Result<String> {
    seal(dk, fk, &ctx_fk(file_id))
}

pub fn open_meta(fk: &RawKey, file_id: &str, enc_meta: &str) -> Result<FileMeta> {
    let plain = open(fk, enc_meta, &ctx_meta(file_id))?;
    serde_json::from_slice(&plain).map_err(|e| format!("métadonnées invalides: {e}"))
}

pub fn seal_meta(fk: &RawKey, file_id: &str, meta: &FileMeta) -> Result<String> {
    let json = serde_json::to_vec(meta).map_err(|e| e.to_string())?;
    seal(fk, &json, &ctx_meta(file_id))
}

pub fn open_folder_name(dk: &RawKey, folder_id: &str, enc_name: &str) -> Result<String> {
    String::from_utf8(open(dk, enc_name, &ctx_folder(folder_id))?).map_err(|e| e.to_string())
}

pub fn seal_folder_name(dk: &RawKey, folder_id: &str, name: &str) -> Result<String> {
    seal(dk, name.as_bytes(), &ctx_folder(folder_id))
}

fn chunk_iv(prefix: &[u8; NONCE_PREFIX_LEN], index: u32, last: bool) -> [u8; 12] {
    let mut iv = [0u8; 12];
    iv[..7].copy_from_slice(prefix);
    iv[7..11].copy_from_slice(&index.to_be_bytes());
    iv[11] = u8::from(last);
    iv
}

pub fn prefix_from_b64(s: &str) -> Result<[u8; NONCE_PREFIX_LEN]> {
    B64.decode(s.trim())
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|_| "noncePrefix de 7 octets attendu".to_string())
}

pub fn encrypt_chunk(
    fk: &RawKey,
    prefix: &[u8; NONCE_PREFIX_LEN],
    file_id: &str,
    index: u32,
    last: bool,
    plain: &[u8],
) -> Result<Vec<u8>> {
    if plain.len() as u64 > CHUNK_PLAIN {
        return Err("chunk trop grand".into());
    }
    let aad = ctx_chunk(file_id, index);
    cipher(fk)
        .encrypt(
            Nonce::from_slice(&chunk_iv(prefix, index, last)),
            Payload {
                msg: plain,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| "chiffrement du chunk échoué".to_string())
}

pub fn decrypt_chunk(
    fk: &RawKey,
    prefix: &[u8; NONCE_PREFIX_LEN],
    file_id: &str,
    index: u32,
    last: bool,
    ct: &[u8],
) -> Result<Vec<u8>> {
    let aad = ctx_chunk(file_id, index);
    cipher(fk)
        .decrypt(
            Nonce::from_slice(&chunk_iv(prefix, index, last)),
            Payload {
                msg: ct,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| format!("chunk {index} illisible (clé ou données altérées)"))
}

/// Encrypt a whole file: one ciphertext chunk per 8 MiB of plaintext (an empty
/// file is one empty, `last`-flagged chunk — same as the web client).
pub fn encrypt_file(
    fk: &RawKey,
    prefix: &[u8; NONCE_PREFIX_LEN],
    file_id: &str,
    plain: &[u8],
) -> Result<Vec<Vec<u8>>> {
    let n = plain.len().div_ceil(CHUNK_PLAIN as usize).max(1);
    (0..n)
        .map(|i| {
            let start = i * CHUNK_PLAIN as usize;
            let end = (start + CHUNK_PLAIN as usize).min(plain.len());
            encrypt_chunk(
                fk,
                prefix,
                file_id,
                i as u32,
                i + 1 == n,
                &plain[start..end],
            )
        })
        .collect()
}

pub fn random_prefix() -> [u8; NONCE_PREFIX_LEN] {
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let mut p = [0u8; NONCE_PREFIX_LEN];
    p.copy_from_slice(&nonce[..NONCE_PREFIX_LEN]);
    p
}

pub fn b64(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_round_trip() {
        let k = random_key();
        let blob = seal(&k, b"hello", "ctx").unwrap();
        assert_eq!(open(&k, &blob, "ctx").unwrap(), b"hello");
        assert!(open(&k, &blob, "other").is_err());
    }

    #[test]
    fn file_round_trip_and_tamper() {
        let fk = random_key();
        let prefix = random_prefix();
        let plain: Vec<u8> = (0..(CHUNK_PLAIN as usize + 10))
            .map(|i| (i % 251) as u8)
            .collect();
        let chunks = encrypt_file(&fk, &prefix, "f", &plain).unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len() as u64, CHUNK_CIPHER);
        let mut out = Vec::new();
        for (i, c) in chunks.iter().enumerate() {
            out.extend(decrypt_chunk(&fk, &prefix, "f", i as u32, i == 1, c).unwrap());
        }
        assert_eq!(out, plain);
        // Truncation: chunk 0 was not sealed as the last one.
        assert!(decrypt_chunk(&fk, &prefix, "f", 0, true, &chunks[0]).is_err());
        // Empty file = one empty last chunk.
        let empty = encrypt_file(&fk, &prefix, "e", &[]).unwrap();
        assert_eq!(empty.len(), 1);
        assert_eq!(empty[0].len() as u64, TAG_LEN);
    }

    // Vectors produced by the web client's own code (`lib/crypto/e2ee`, Node
    // WebCrypto) — prove byte parity with the browser.
    const DK: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
    const FILE_ID: &str = "AbCdEfGhIjKlMnOpQrStU";
    include!("e2ee_vectors.rs");

    #[test]
    fn parity_with_web_client() {
        let dk = key_from_b64(DK).unwrap();
        let fk = unwrap_file_key(&dk, FILE_ID, FK_WRAPPED).unwrap();
        let meta = open_meta(&fk, FILE_ID, ENC_META).unwrap();
        assert_eq!(meta.name, "Été 2026 — photo.jpg");
        assert_eq!(meta.mime, "image/jpeg");
        assert_eq!(meta.size, 5);
        assert_eq!(
            open_folder_name(&dk, "folder123", ENC_FOLDER).unwrap(),
            "Vacances ✈"
        );
        let prefix = prefix_from_b64(NONCE_PREFIX).unwrap();
        let ct = B64.decode(CHUNK0).unwrap();
        assert_eq!(
            decrypt_chunk(&fk, &prefix, FILE_ID, 0, true, &ct).unwrap(),
            b"hello"
        );
        // And the other way round: what we produce, the web client's format accepts.
        let again = encrypt_chunk(&fk, &prefix, FILE_ID, 0, true, b"hello").unwrap();
        assert_eq!(again, ct);
    }
}
