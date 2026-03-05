use crate::models::{AppError, AppResult, ChunkDescriptor};
use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    AeadCore, Aes256Gcm, Nonce,
};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

#[derive(Clone)]
pub struct ChunkingEngine {
    chunk_size: usize,
    encryption_key: [u8; 32],
    cipher: Aes256Gcm,
}

impl ChunkingEngine {
    pub fn new(chunk_size: usize, encryption_key: [u8; 32]) -> Self {
        let cipher = Aes256Gcm::new_from_slice(&encryption_key).expect("invalid aes key length");
        Self {
            chunk_size,
            encryption_key,
            cipher,
        }
    }

    pub fn chunk_size(&self) -> usize {
        self.chunk_size
    }

    pub fn with_chunk_size(&self, chunk_size: usize) -> Self {
        Self::new(chunk_size, self.encryption_key)
    }

    pub async fn hash_file(&self, path: &Path) -> AppResult<(String, u64)> {
        self.hash_file_with_progress(path, |_, _| Ok(())).await
    }

    pub async fn hash_file_with_progress<F>(
        &self,
        path: &Path,
        mut on_progress: F,
    ) -> AppResult<(String, u64)>
    where
        F: FnMut(u64, u64) -> AppResult<()>,
    {
        let mut file = File::open(path).await?;
        let total_size = file.metadata().await?.len();
        let mut buffer = vec![0u8; self.chunk_size.max(64 * 1024)];
        let mut file_hasher = Sha256::new();
        let mut processed = 0u64;

        loop {
            let n = file.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            file_hasher.update(&buffer[..n]);
            processed += n as u64;
            on_progress(processed, total_size)?;
        }

        Ok((hex::encode(file_hasher.finalize()), processed))
    }

    pub async fn split_and_encrypt_file(
        &self,
        path: &Path,
    ) -> AppResult<(String, u64, Vec<ChunkDescriptor>)> {
        self.split_and_encrypt_file_with_progress(path, |_, _| Ok(()))
            .await
    }

    pub async fn split_and_encrypt_file_with_progress<F>(
        &self,
        path: &Path,
        mut on_progress: F,
    ) -> AppResult<(String, u64, Vec<ChunkDescriptor>)>
    where
        F: FnMut(u64, u64) -> AppResult<()>,
    {
        let mut file = File::open(path).await?;
        let total_size_hint = file.metadata().await?.len();
        let mut buffer = vec![0u8; self.chunk_size];
        let mut file_hasher = Sha256::new();
        let mut parts = Vec::new();
        let mut index: i64 = 0;
        let mut total_size: u64 = 0;

        loop {
            let n = file.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            let chunk_plain = &buffer[..n];
            total_size += n as u64;
            file_hasher.update(chunk_plain);

            let mut chunk_hasher = Sha256::new();
            chunk_hasher.update(chunk_plain);
            let chunk_hash = hex::encode(chunk_hasher.finalize());

            let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
            let encrypted = self
                .cipher
                .encrypt(&nonce, chunk_plain)
                .map_err(|e| AppError::Crypto(format!("chunk encryption failed: {e}")))?;

            parts.push(ChunkDescriptor {
                part_index: index,
                hash: chunk_hash,
                size: n,
                nonce_b64: base64::engine::general_purpose::STANDARD.encode(nonce),
                bytes: encrypted,
            });
            index += 1;
            on_progress(total_size, total_size_hint)?;
        }

        Ok((hex::encode(file_hasher.finalize()), total_size, parts))
    }

    pub fn decrypt_chunk(&self, nonce_b64: &str, encrypted: &[u8]) -> AppResult<Vec<u8>> {
        let nonce_bytes = base64::engine::general_purpose::STANDARD
            .decode(nonce_b64)
            .map_err(|e| AppError::Crypto(format!("nonce decode failed: {e}")))?;
        if nonce_bytes.len() != 12 {
            return Err(AppError::Crypto("invalid nonce length".to_string()));
        }

        let nonce = Nonce::from_slice(&nonce_bytes);
        self.cipher
            .decrypt(nonce, encrypted)
            .map_err(|e| AppError::Crypto(format!("chunk decrypt failed: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn aes_roundtrip() {
        let e = ChunkingEngine::new(8, [7u8; 32]);
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let source = b"telegram-drive";
        let encrypted = e.cipher.encrypt(&nonce, source.as_ref()).unwrap();
        let out = e
            .decrypt_chunk(
                &base64::engine::general_purpose::STANDARD.encode(nonce),
                &encrypted,
            )
            .unwrap();
        assert_eq!(source.to_vec(), out);
    }
}
