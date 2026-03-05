use crate::cache::LocalCdnCache;
use crate::chunking::ChunkingEngine;
use crate::database::{Database, NewChunkRecord, NewFileRecord};
use crate::dedup::DedupEngine;
use crate::models::{classify_mime, AppError, AppResult, TransferState};
use crate::progress::ProgressHub;
use crate::telegram::TelegramClient;
use futures::stream::{FuturesUnordered, StreamExt};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{error, info};
use uuid::Uuid;

#[derive(Clone)]
pub struct UploadService {
    db: Database,
    dedup: DedupEngine,
    chunking: ChunkingEngine,
    telegram: TelegramClient,
    cache: LocalCdnCache,
    progress: ProgressHub,
}

impl UploadService {
    pub fn new(
        db: Database,
        dedup: DedupEngine,
        chunking: ChunkingEngine,
        telegram: TelegramClient,
        cache: LocalCdnCache,
        progress: ProgressHub,
    ) -> Self {
        Self {
            db,
            dedup,
            chunking,
            telegram,
            cache,
            progress,
        }
    }

    pub async fn upload_file(&self, folder_id: i64, file_path: PathBuf, max_parallelism: usize) -> AppResult<i64> {
        let job_id = Uuid::new_v4().to_string();
        let file_name = file_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        self.progress
            .transition(&job_id, &file_name, TransferState::Queued, 0, 0, None);

        let (file_hash, size, mut chunks) = self.chunking.split_and_encrypt_file(&file_path).await?;
        self.progress
            .transition(&job_id, &file_name, TransferState::Running, 0, size, None);

        if let Some(existing) = self.dedup.find_duplicate_file(&file_hash)? {
            let destination_name = self.db.resolve_conflict_name(folder_id, &file_name)?;
            let new_id = self
                .db
                .create_file_reference(existing.id, folder_id, &destination_name)?;
            self.progress.transition(
                &job_id,
                &file_name,
                TransferState::Completed,
                size,
                size,
                None,
            );
            info!(file = %file_name, "file-level dedup hit; created reference");
            return Ok(new_id);
        }

        let adaptive = compute_parallelism(max_parallelism);
        let semaphore = Arc::new(Semaphore::new(adaptive));
        let mut futures = FuturesUnordered::new();
        let mut uploaded = Vec::<NewChunkRecord>::new();
        let mut processed_bytes = 0u64;

        for descriptor in chunks.drain(..) {
            if self.progress.is_cancelled(&job_id) {
                self.progress.transition(
                    &job_id,
                    &file_name,
                    TransferState::Cancelled,
                    processed_bytes,
                    size,
                    None,
                );
                return Err(AppError::Validation("upload cancelled".to_string()));
            }

            if let Some(existing) = self
                .dedup
                .find_duplicate_chunk(&descriptor.hash, descriptor.size as i64)?
            {
                processed_bytes += descriptor.size as u64;
                uploaded.push(NewChunkRecord {
                    part_index: descriptor.part_index,
                    hash: descriptor.hash.clone(),
                    telegram_file_id: existing.telegram_file_id,
                    size: descriptor.size as i64,
                    nonce_b64: descriptor.nonce_b64,
                });
                self.progress.transition(
                    &job_id,
                    &file_name,
                    TransferState::Running,
                    processed_bytes,
                    size,
                    None,
                );
                continue;
            }

            let permit = semaphore.clone().acquire_owned().await.map_err(|_| {
                AppError::Concurrency("failed acquiring upload worker semaphore".to_string())
            })?;
            let telegram = self.telegram.clone();
            let cache = self.cache.clone();
            futures.push(tokio::spawn(async move {
                let _permit = permit;
                let telegram_id = telegram.upload_chunk(descriptor.bytes.clone()).await?;
                cache.write_chunk(&descriptor.hash, &descriptor.bytes).await?;
                Ok::<NewChunkRecord, AppError>(NewChunkRecord {
                    part_index: descriptor.part_index,
                    hash: descriptor.hash,
                    telegram_file_id: telegram_id,
                    size: descriptor.size as i64,
                    nonce_b64: descriptor.nonce_b64,
                })
            }));
        }

        while let Some(result) = futures.next().await {
            match result {
                Ok(Ok(chunk_row)) => {
                    processed_bytes += chunk_row.size as u64;
                    uploaded.push(chunk_row);
                    self.progress.transition(
                        &job_id,
                        &file_name,
                        TransferState::Running,
                        processed_bytes,
                        size,
                        None,
                    );
                }
                Ok(Err(e)) => {
                    error!(error = %e, "chunk upload failed");
                    self.progress.transition(
                        &job_id,
                        &file_name,
                        TransferState::Failed,
                        processed_bytes,
                        size,
                        Some(e.to_string()),
                    );
                    return Err(e);
                }
                Err(join_err) => {
                    let msg = format!("upload worker join error: {join_err}");
                    self.progress.transition(
                        &job_id,
                        &file_name,
                        TransferState::Failed,
                        processed_bytes,
                        size,
                        Some(msg.clone()),
                    );
                    return Err(AppError::Concurrency(msg));
                }
            }
        }

        uploaded.sort_by_key(|c| c.part_index);
        let safe_name = self.db.resolve_conflict_name(folder_id, &file_name)?;
        let file_id = self.db.persist_uploaded_file(
            NewFileRecord {
                name: safe_name,
                size: size as i64,
                hash: file_hash,
                folder_id,
                mime_type: classify_mime(file_path.as_path()),
                original_path: Some(file_path.to_string_lossy().to_string()),
            },
            uploaded,
        )?;

        self.progress
            .transition(&job_id, &file_name, TransferState::Completed, size, size, None);
        Ok(file_id)
    }
}

fn compute_parallelism(max_parallelism: usize) -> usize {
    let cpu = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let adaptive = (cpu * 2).clamp(2, 32);
    adaptive.min(max_parallelism.max(1))
}
