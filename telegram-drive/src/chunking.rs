use crate::models::{AppError, AppResult, ChunkDescriptor, TransferPhase};
use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    AeadCore, Aes256Gcm, Nonce,
};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, BufReader};

// Tamanho do buffer interno do BufReader — 2× o chunk para minimizar syscalls
// de I/O sem desperdiçar memória.
const IO_BUFFER_MULTIPLIER: usize = 2;

#[derive(Debug, Clone)]
pub struct ChunkPipelineProgress {
    pub phase: TransferPhase,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

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
        let file = File::open(path).await?;
        let total_size = file.metadata().await?.len();
        // BufReader com buffer grande reduz syscalls ao percorrer arquivos pesados
        let buf_cap = self.chunk_size.max(64 * 1024) * IO_BUFFER_MULTIPLIER;
        let mut reader = BufReader::with_capacity(buf_cap, file);
        let mut buffer = vec![0u8; self.chunk_size.max(64 * 1024)];
        let mut file_hasher = Sha256::new();
        let mut processed = 0u64;

        loop {
            // read_buf_exact não existe no tokio; usamos um loop de read para
            // garantir que o buffer seja preenchido antes de atualizar o hash.
            let n = read_full(&mut reader, &mut buffer).await?;
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

    /// Lê o arquivo em chunks, criptografa cada um e retorna a lista completa.
    ///
    /// **Atenção:** para arquivos muito grandes (>2 GiB) prefira
    /// [`stream_and_encrypt_chunks`] que evita acumular todos os bytes na RAM.
    pub async fn split_and_encrypt_file_with_progress<F>(
        &self,
        path: &Path,
        mut on_progress: F,
    ) -> AppResult<(String, u64, Vec<ChunkDescriptor>)>
    where
        F: FnMut(u64, u64) -> AppResult<()>,
    {
        let file = File::open(path).await?;
        let total_size_hint = file.metadata().await?.len();
        let buf_cap = self.chunk_size * IO_BUFFER_MULTIPLIER;
        let mut reader = BufReader::with_capacity(buf_cap, file);
        let mut buffer = vec![0u8; self.chunk_size];
        let mut file_hasher = Sha256::new();
        let mut parts = Vec::new();
        let mut index: i64 = 0;
        let mut total_size: u64 = 0;

        loop {
            // Garante buffer completamente preenchido antes de criptografar.
            // Sem isso, `file.read()` pode retornar muito menos bytes do que
            // o chunk_size, gerando milhares de chunks minúsculos para arquivos
            // grandes e tornando o processo muito lento.
            let n = read_full(&mut reader, &mut buffer).await?;
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

    /// Pipeline de streaming: produz chunks criptografados **um a um** através
    /// de um channel, sem acumular a totalidade do arquivo na memória.
    ///
    /// Retorna `(file_hash_receiver, total_bytes_receiver, chunk_receiver)`.
    ///
    /// O chamador consome `chunk_rx` à medida que faz upload de cada chunk;
    /// `file_hash_rx` e `total_rx` só têm valor após o channel fechar.
    ///
    /// # Como funciona
    /// ```text
    /// [disco] ──read_full──► [encrypt] ──► chunk_tx ──► uploader (em paralelo)
    ///                                  └──► file_hasher ──► file_hash_tx
    /// ```
    pub fn stream_and_encrypt_chunks(
        &self,
        path: std::path::PathBuf,
        total_size_hint: u64,
    ) -> (
        tokio::sync::oneshot::Receiver<AppResult<(String, u64)>>,
        tokio::sync::mpsc::Receiver<ChunkPipelineProgress>,
        tokio::sync::mpsc::Receiver<AppResult<ChunkDescriptor>>,
    ) {
        // Canal de chunks com backpressure: no m?ximo `PIPELINE_BUFFER` chunks
        // aguardando upload simultaneamente, evitando ocupar RAM em excesso.
        const PIPELINE_BUFFER: usize = 8;
        let (chunk_tx, chunk_rx) = tokio::sync::mpsc::channel(PIPELINE_BUFFER);
        let (progress_tx, progress_rx) = tokio::sync::mpsc::channel(PIPELINE_BUFFER * 2);
        let (meta_tx, meta_rx) = tokio::sync::oneshot::channel();

        let engine = self.clone();

        tokio::spawn(async move {
            let result = engine
                .produce_chunks(path, total_size_hint, progress_tx, chunk_tx)
                .await;
            let _ = meta_tx.send(result);
        });

        (meta_rx, progress_rx, chunk_rx)
    }

    /// Tarefa interna do pipeline: lê o arquivo e envia chunks pelo canal.
    async fn produce_chunks(
        &self,
        path: std::path::PathBuf,
        total_size_hint: u64,
        progress_tx: tokio::sync::mpsc::Sender<ChunkPipelineProgress>,
        chunk_tx: tokio::sync::mpsc::Sender<AppResult<ChunkDescriptor>>,
    ) -> AppResult<(String, u64)> {
        let file = File::open(&path).await?;
        let buf_cap = self.chunk_size * IO_BUFFER_MULTIPLIER;
        let mut reader = BufReader::with_capacity(buf_cap, file);
        let mut buffer = vec![0u8; self.chunk_size];
        let mut file_hasher = Sha256::new();
        let mut index: i64 = 0;
        let mut total_size: u64 = 0;

        loop {
            let n = read_full(&mut reader, &mut buffer).await?;
            if n == 0 {
                break;
            }
            let chunk_plain = &buffer[..n];
            total_size += n as u64;
            let _ = progress_tx
                .send(ChunkPipelineProgress {
                    phase: TransferPhase::Chunking,
                    bytes_done: total_size,
                    bytes_total: total_size_hint,
                })
                .await;
            file_hasher.update(chunk_plain);

            let mut chunk_hasher = Sha256::new();
            chunk_hasher.update(chunk_plain);
            let chunk_hash = hex::encode(chunk_hasher.finalize());

            let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
            let encrypted = self
                .cipher
                .encrypt(&nonce, chunk_plain)
                .map_err(|e| AppError::Crypto(format!("chunk encryption failed: {e}")))?;
            let _ = progress_tx
                .send(ChunkPipelineProgress {
                    phase: TransferPhase::Encrypting,
                    bytes_done: total_size,
                    bytes_total: total_size_hint,
                })
                .await;

            let descriptor = ChunkDescriptor {
                part_index: index,
                hash: chunk_hash,
                size: n,
                nonce_b64: base64::engine::general_purpose::STANDARD.encode(nonce),
                bytes: encrypted,
            };

            // Se o receiver foi descartado (cancelamento), para a produção.
            if chunk_tx.send(Ok(descriptor)).await.is_err() {
                return Err(AppError::Validation("upload cancelled".to_string()));
            }

            index += 1;

            // Emite progresso aproximado via tracing para não precisar de
            // callback aqui (o uploader controla o progresso externamente).
            tracing::trace!(
                processed = total_size,
                total = total_size_hint,
                chunk_index = index,
                "chunk produced"
            );
        }

        Ok((hex::encode(file_hasher.finalize()), total_size))
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

/// Lê bytes suficientes para preencher `buf` completamente, ou até EOF.
///
/// O `AsyncReadExt::read()` padrão pode retornar bem menos bytes do que o
/// buffer comporta em uma única chamada — especialmente em arquivos grandes —
/// causando chunks minúsculos e milhares de operações desnecessárias.
/// Esta função faz chamadas repetidas até o buffer estar cheio ou o arquivo
/// ter terminado, garantindo o tamanho correto de cada chunk.
async fn read_full<R: AsyncReadExt + Unpin>(reader: &mut R, buf: &mut [u8]) -> AppResult<usize> {
    let mut total = 0;
    while total < buf.len() {
        let n = reader.read(&mut buf[total..]).await.map_err(AppError::Io)?;
        if n == 0 {
            break; // EOF
        }
        total += n;
    }
    Ok(total)
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

    #[tokio::test]
    async fn read_full_fills_buffer() {
        // Simula um leitor que retorna apenas 1 byte por vez (pior caso)
        use std::pin::Pin;
        use std::task::{Context, Poll};
        use tokio::io::AsyncRead;

        struct OneByte(Vec<u8>, usize);
        impl AsyncRead for OneByte {
            fn poll_read(
                mut self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                buf: &mut tokio::io::ReadBuf<'_>,
            ) -> Poll<std::io::Result<()>> {
                if self.1 >= self.0.len() {
                    return Poll::Ready(Ok(()));
                }
                buf.put_slice(&self.0[self.1..self.1 + 1]);
                self.1 += 1;
                Poll::Ready(Ok(()))
            }
        }

        let data: Vec<u8> = (0..64).collect();
        let mut reader = OneByte(data.clone(), 0);
        let mut buf = vec![0u8; 64];
        let n = read_full(&mut reader, &mut buf).await.unwrap();
        assert_eq!(n, 64);
        assert_eq!(&buf[..n], &data[..]);
    }

    #[tokio::test]
    async fn stream_pipeline_produces_correct_chunks() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let mut tmp = NamedTempFile::new().unwrap();
        // Escreve 3 chunks completos + 1 chunk parcial
        let chunk_size = 1024usize;
        let total_bytes = chunk_size * 3 + 512;
        let data: Vec<u8> = (0..total_bytes).map(|i| (i % 251) as u8).collect();
        tmp.write_all(&data).unwrap();
        tmp.flush().unwrap();

        let key = [0u8; 32];
        let engine = ChunkingEngine::new(chunk_size, key);
        let path = tmp.path().to_path_buf();
        let total_hint = total_bytes as u64;

        let (meta_rx, _progress_rx, mut chunk_rx) =
            engine.stream_and_encrypt_chunks(path.clone(), total_hint);

        let mut chunks = Vec::new();
        while let Some(res) = chunk_rx.recv().await {
            chunks.push(res.expect("chunk error"));
        }

        let (file_hash, total_size) = meta_rx.await.unwrap().unwrap();

        // Verifica contagem de chunks
        assert_eq!(chunks.len(), 4, "deve gerar 4 chunks");
        assert_eq!(total_size, total_bytes as u64);

        // Verifica que descriptografar cada chunk reproduz os dados originais
        for chunk in &chunks {
            let plain = engine
                .decrypt_chunk(&chunk.nonce_b64, &chunk.bytes)
                .expect("decrypt failed");
            let start = chunk.part_index as usize * chunk_size;
            let end = start + chunk.size;
            assert_eq!(&plain[..], &data[start..end]);
        }

        // Verifica hash do arquivo inteiro
        let (expected_hash, _) = engine.hash_file(&path).await.unwrap();
        assert_eq!(file_hash, expected_hash);
    }
}
