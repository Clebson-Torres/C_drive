use crate::cache::LocalCdnCache;
use crate::chunking::ChunkingEngine;
use crate::database::Database;
use crate::models::{AppError, AppResult, PreviewResponse, TransferState};
use crate::progress::ProgressHub;
use crate::telegram::TelegramClient;
use futures::stream::{FuturesUnordered, StreamExt};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use tracing::error;
use uuid::Uuid;

#[derive(Clone)]
pub struct DownloadService {
    db: Database,
    chunking: ChunkingEngine,
    telegram: TelegramClient,
    cache: LocalCdnCache,
    progress: ProgressHub,
}

impl DownloadService {
    pub fn new(
        db: Database,
        chunking: ChunkingEngine,
        telegram: TelegramClient,
        cache: LocalCdnCache,
        progress: ProgressHub,
    ) -> Self {
        Self {
            db,
            chunking,
            telegram,
            cache,
            progress,
        }
    }

    pub async fn download_file(&self, file_id: i64, destination_path: PathBuf, max_parallelism: usize) -> AppResult<()> {
        let file = self.db.get_file(file_id)?;
        let chunks = self.db.get_chunks_for_file(file_id)?;

        let job_id = Uuid::new_v4().to_string();
        let total = file.size.max(0) as u64;
        self.progress
            .transition(&job_id, &file.name, TransferState::Running, 0, total, None);

        let semaphore = Arc::new(Semaphore::new(compute_parallelism(max_parallelism)));
        let mut futures = FuturesUnordered::new();

        for (part_index, hash, telegram_file_id, _size, nonce_b64) in chunks {
            let cache = self.cache.clone();
            let tg = self.telegram.clone();
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| AppError::Concurrency("download semaphore closed".to_string()))?;
            futures.push(tokio::spawn(async move {
                let _permit = permit;
                if let Some(bytes) = cache.read_chunk(&hash).await? {
                    return Ok::<(i64, String, String, Vec<u8>), AppError>((part_index, hash, nonce_b64, bytes));
                }
                let bytes = tg.download_chunk(&telegram_file_id).await?;
                cache.write_chunk(&hash, &bytes).await?;
                Ok((part_index, hash, nonce_b64, bytes))
            }));
        }

        let mut ordered = BTreeMap::<i64, Vec<u8>>::new();
        let mut done = 0u64;

        while let Some(next) = futures.next().await {
            match next {
                Ok(Ok((part_index, hash, nonce_b64, encrypted))) => {
                    let decrypted = self.chunking.decrypt_chunk(&nonce_b64, &encrypted)?;
                    let mut hasher = Sha256::new();
                    hasher.update(&decrypted);
                    let digest = hex::encode(hasher.finalize());
                    if digest != hash {
                        return Err(AppError::Validation(format!(
                            "hash mismatch in part {} expected={} got={}",
                            part_index, hash, digest
                        )));
                    }
                    done += decrypted.len() as u64;
                    ordered.insert(part_index, decrypted);
                    self.progress
                        .transition(&job_id, &file.name, TransferState::Running, done, total, None);
                }
                Ok(Err(e)) => {
                    self.progress.transition(
                        &job_id,
                        &file.name,
                        TransferState::Failed,
                        done,
                        total,
                        Some(e.to_string()),
                    );
                    return Err(e);
                }
                Err(join_err) => {
                    let msg = format!("download worker join error: {join_err}");
                    error!(%msg);
                    self.progress.transition(
                        &job_id,
                        &file.name,
                        TransferState::Failed,
                        done,
                        total,
                        Some(msg.clone()),
                    );
                    return Err(AppError::Concurrency(msg));
                }
            }
        }

        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let partial = destination_path.with_extension("partial");
        write_reassembled_file(&partial, &ordered).await?;
        fs::rename(&partial, &destination_path).await?;

        self.progress
            .transition(&job_id, &file.name, TransferState::Completed, total, total, None);
        Ok(())
    }

    pub async fn materialize_preview(&self, file_id: i64) -> AppResult<PreviewResponse> {
        let file = self.db.get_file(file_id)?;
        if !file.mime_type.starts_with("image/") {
            return Err(AppError::Validation("preview is only available for image files".to_string()));
        }

        let ext = Path::new(&file.name)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("bin");
        let temp = std::env::temp_dir().join(format!("telegram-drive-preview-{}.{}", file.id, ext));
        self.download_file(file_id, temp.clone(), 4).await?;

        Ok(PreviewResponse {
            local_path: temp.to_string_lossy().to_string(),
            mime_type: file.mime_type,
        })
    }
}

async fn write_reassembled_file(path: &Path, parts: &BTreeMap<i64, Vec<u8>>) -> AppResult<()> {
    let mut out = File::create(path).await?;
    for part in parts.values() {
        out.write_all(part).await?;
    }
    out.flush().await?;
    Ok(())
}

fn compute_parallelism(max_parallelism: usize) -> usize {
    let cpu = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let adaptive = (cpu * 2).clamp(2, 32);
    adaptive.min(max_parallelism.max(1))
}
