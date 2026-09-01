//! AES-256-GCM, byte-compatible with the web client's `crypto.subtle`
//! (`src/lib/crypto/vault-crypto.ts`): random 12-byte IV stored separately as
//! `encIv` (base64), ciphertext laid out as `ciphertext ‖ 16-byte tag`.

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    AeadCore, Aes256Gcm, Key, Nonce,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};

pub type Result<T> = std::result::Result<T, String>;

fn key_from_b64(key_b64: &str) -> Result<Aes256Gcm> {
    let raw = B64
        .decode(key_b64.trim())
        .map_err(|e| format!("clé base64 invalide: {e}"))?;
    if raw.len() != 32 {
        return Err(format!("clé de {} octets, 32 attendus", raw.len()));
    }
    Ok(Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&raw)))
}

/// Decrypt `ciphertext` (`ct ‖ tag`) with the drive key + stored IV.
pub fn decrypt(key_b64: &str, iv_b64: &str, ciphertext: &[u8]) -> Result<Vec<u8>> {
    let cipher = key_from_b64(key_b64)?;
    let iv = B64
        .decode(iv_b64.trim())
        .map_err(|e| format!("IV base64 invalide: {e}"))?;
    if iv.len() != 12 {
        return Err(format!("IV de {} octets, 12 attendus", iv.len()));
    }
    cipher
        .decrypt(Nonce::from_slice(&iv), ciphertext)
        .map_err(|_| "déchiffrement AES-GCM échoué (clé ou données corrompues)".to_string())
}

/// Encrypt `plain`. Returns `(iv_b64, ciphertext ‖ tag)` — the `iv_b64` goes to
/// the `encIv` column, the bytes get chunked and uploaded.
pub fn encrypt(key_b64: &str, plain: &[u8]) -> Result<(String, Vec<u8>)> {
    let cipher = key_from_b64(key_b64)?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ct = cipher
        .encrypt(&nonce, plain)
        .map_err(|_| "chiffrement AES-GCM échoué".to_string())?;
    Ok((B64.encode(nonce), ct))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Vector produced by Node's WebCrypto (see PR notes) — proves byte parity
    // with the browser upload path.
    const KEY: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";
    const IV: &str = "qrvM3e7/ABEiM0RV";
    const PLAIN_HEX: &str = "4472697665636f72642073796e63204532454520706172697479207465737420e2809420616363656e74733a20c3a9c3a0c3a7c3bc20f09f9a80";
    const CT: &str = "o0Uuio1nVd8b8cGGlKM8zcKQ3VE3PXM3nSKEOqp+ZljKKIMO+dbgyG2MTL06bKRGln3Lujlk5qKgWg75xX+iwX47VGI42hsa+RY=";

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn decrypts_webcrypto_ciphertext() {
        let ct = B64.decode(CT).unwrap();
        let out = decrypt(KEY, IV, &ct).unwrap();
        assert_eq!(out, unhex(PLAIN_HEX));
    }

    #[test]
    fn round_trip() {
        let plain = unhex(PLAIN_HEX);
        let (iv, ct) = encrypt(KEY, &plain).unwrap();
        assert_eq!(decrypt(KEY, &iv, &ct).unwrap(), plain);
    }
}
