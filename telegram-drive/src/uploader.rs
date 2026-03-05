use crate::cache::LocalCdnCache;
use crate::chunking::ChunkingEngine;
use crate::database::{Database, NewChunkRecord, NewFileRecord};
use crate::dedup::DedupEngine;
use crate::models::{
    classify_mime, normalize_chunk_size_bytes, AppError, AppResult, StorageMode, TransferPhase,
    TransferState,
};
use crate::progress::ProgressHub;
use crate::telegram::TelegramClient;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{error, info, info_span, Instrument};
use uuid::Uuid;

pub const SINGLE_UPLOAD_THRESHOLD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
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

    pub async fn upload_file(
        &self,
        folder_id: i64,
        file_path: PathBuf,
        max_parallelism: usize,
        chunk_size_bytes: usize,
    ) -> AppResult<i64> {
        let job_id = Uuid::new_v4().to_string();
        let file_name = file_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let metadata = tokio::fs::metadata(&file_path).await?;
        let file_size = metadata.len();
        let storage_mode = storage_mode_for_size(file_size);

        self.progress.transition(
            &job_id,
            &file_name,
            TransferState::Queued,
            TransferPhase::Queued,
            Some(storage_mode.clone()),
            0,
            file_size,
            None,
        );
        self.progress.transition(
            &job_id,
            &file_name,
            TransferState::Running,
            TransferPhase::Hashing,
            Some(storage_mode.clone()),
            0,
            file_size,
            None,
        );

        let span = info_span!(
            "upload_file",
            job_id = %job_id,
            file = %file_path.display(),
            bytes_total = file_size,
            storage_mode = ?storage_mode
        );

        async move {
            match storage_mode {
                StorageMode::Single => {
                    let (file_hash, hashed_size) = self
                        .chunking
                        .hash_file_with_progress(&file_path, |done, total| {
                            if self.progress.is_cancelled(&job_id) {
                                return Err(AppError::Validation("upload cancelled".to_string()));
                            }
                            self.progress.transition(
                                &job_id,
                                &file_name,
                                TransferState::Running,
                                TransferPhase::Hashing,
                                Some(StorageMode::Single),
                                done,
                                total,
                                None,
                            );
                            Ok(())
                        })
                        .await
                        .map_err(|e| {
                            let is_cancel = e.to_string().contains("upload cancelled");
                            self.progress.transition(
                                &job_id,
                                &file_name,
                                if is_cancel {
                                    TransferState::Cancelled
                                } else {
                                    TransferState::Failed
                                },
                                if is_cancel {
                                    TransferPhase::Cancelled
                                } else {
                                    TransferPhase::Failed
                                },
                                Some(StorageMode::Single),
                                0,
                                file_size,
                                Some(e.to_string()),
                            );
                            e
                        })?;

                    if let Some(existing) = self.dedup.find_duplicate_file(&file_hash)? {
                        let destination_name =
                            self.db.resolve_conflict_name(folder_id, &file_name)?;
                        let new_id = self.db.create_file_reference(
                            existing.id,
                            folder_id,
                            &destination_name,
                        )?;
                        self.progress.transition(
                            &job_id,
                            &file_name,
                            TransferState::Completed,
                            TransferPhase::Completed,
                            Some(StorageMode::Single),
                            hashed_size,
                            hashed_size,
                            None,
                        );
                        info!(file = %file_name, "file-level dedup hit; created reference");
                        return Ok(new_id);
                    }

                    self.upload_single_file(
                        &job_id,
                        folder_id,
                        file_path,
                        file_name,
                        file_hash,
                        hashed_size,
                    )
                    .await
                }
                StorageMode::Chunked => {
                    self.upload_chunked_file(
                        &job_id,
                        folder_id,
                        file_path,
                        file_name,
                        max_parallelism,
                        chunk_size_bytes,
                    )
                    .await
                }
            }
        }
        .instrument(span)
        .await
    }

    async fn upload_single_file(
        &self,
        job_id: &str,
        folder_id: i64,
        file_path: PathBuf,
        file_name: String,
        file_hash: String,
        file_size: u64,
    ) -> AppResult<i64> {
        self.progress.transition(
            job_id,
            &file_name,
            TransferState::Running,
            TransferPhase::Uploading,
            Some(StorageMode::Single),
            0,
            file_size,
            None,
        );

        let telegram_file_id = self
            .telegram
            .upload_file_path(&file_path)
            .await
            .map_err(|e| {
                self.progress.transition(
                    job_id,
                    &file_name,
                    TransferState::Failed,
                    TransferPhase::Failed,
                    Some(StorageMode::Single),
                    0,
                    file_size,
                    Some(e.to_string()),
                );
                e
            })?;

        self.cache.import_file(&file_hash, &file_path).await?;

        let safe_name = self.db.resolve_conflict_name(folder_id, &file_name)?;
        let file_id = self.db.persist_uploaded_file(
            NewFileRecord {
                name: safe_name,
                size: file_size as i64,
                hash: file_hash.clone(),
                folder_id,
                mime_type: classify_mime(file_path.as_path()),
                original_path: Some(file_path.to_string_lossy().to_string()),
                storage_mode: StorageMode::Single,
                telegram_file_id: Some(telegram_file_id),
            },
            Vec::new(),
        )?;

        self.progress.transition(
            job_id,
            &file_name,
            TransferState::Completed,
            TransferPhase::Completed,
            Some(StorageMode::Single),
            file_size,
            file_size,
            None,
        );
        Ok(file_id)
    }

    async fn upload_chunked_file(
        &self,
        job_id: &str,
        folder_id: i64,
        file_path: PathBuf,
        file_name: String,
        max_parallelism: usize,
        chunk_size_bytes: usize,
    ) -> AppResult<i64> {
        let file_size = tokio::fs::metadata(&file_path).await?.len();
        let effective_chunk_size = normalize_chunk_size_bytes(chunk_size_bytes);
        let chunking = self.chunking.with_chunk_size(effective_chunk_size);
        self.progress.transition(
            job_id,
            &file_name,
            TransferState::Running,
            TransferPhase::Uploading,
            Some(StorageMode::Chunked),
            0,
            file_size,
            None,
        );

        let (file_hash, actual_size, mut chunks) = chunking
            .split_and_encrypt_file_with_progress(&file_path, |done, total| {
                if self.progress.is_cancelled(job_id) {
                    return Err(AppError::Validation("upload cancelled".to_string()));
                }
                self.progress.transition(
                    job_id,
                    &file_name,
                    TransferState::Running,
                    TransferPhase::Uploading,
                    Some(StorageMode::Chunked),
                    done,
                    total,
                    None,
                );
                Ok(())
            })
            .await
            .map_err(|e| {
                let is_cancel = e.to_string().contains("upload cancelled");
                self.progress.transition(
                    job_id,
                    &file_name,
                    if is_cancel {
                        TransferState::Cancelled
                    } else {
                        TransferState::Failed
                    },
                    if is_cancel {
                        TransferPhase::Cancelled
                    } else {
                        TransferPhase::Failed
                    },
                    Some(StorageMode::Chunked),
                    0,
                    file_size,
                    Some(e.to_string()),
                );
                e
            })?;
        let file_size = actual_size;

        if let Some(existing) = self.dedup.find_duplicate_file(&file_hash)? {
            let destination_name = self.db.resolve_conflict_name(folder_id, &file_name)?;
            let new_id =
                self.db
                    .create_file_reference(existing.id, folder_id, &destination_name)?;
            self.progress.transition(
                job_id,
                &file_name,
                TransferState::Completed,
                TransferPhase::Completed,
                Some(StorageMode::Chunked),
                file_size,
                file_size,
                None,
            );
            info!(file = %file_name, "file-level dedup hit after chunk preparation; created reference");
            return Ok(new_id);
        }

        let adaptive = compute_parallelism(max_parallelism);
        let semaphore = Arc::new(Semaphore::new(adaptive));
        let mut uploads = JoinSet::new();
        let mut uploaded = Vec::<NewChunkRecord>::new();
        let mut processed_bytes = 0u64;
        let mut unique_descriptors = HashMap::new();
        let mut parts_by_hash = HashMap::<String, Vec<i64>>::new();
        let mut resolved_by_hash = HashMap::<String, (String, String, i64)>::new();
        let mut nonce_by_hash = HashMap::<String, String>::new();

        for descriptor in chunks.drain(..) {
            if self.progress.is_cancelled(job_id) {
                self.progress.transition(
                    job_id,
                    &file_name,
                    TransferState::Cancelled,
                    TransferPhase::Cancelled,
                    Some(StorageMode::Chunked),
                    processed_bytes,
                    file_size,
                    None,
                );
                return Err(AppError::Validation("upload cancelled".to_string()));
            }

            parts_by_hash
                .entry(descriptor.hash.clone())
                .or_default()
                .push(descriptor.part_index);

            if unique_descriptors.contains_key(&descriptor.hash)
                || resolved_by_hash.contains_key(&descriptor.hash)
            {
                continue;
            }

            if let Some(existing) = self
                .dedup
                .find_duplicate_chunk(&descriptor.hash, descriptor.size as i64)?
            {
                if existing.nonce_b64.is_empty() {
                    return Err(AppError::Validation(format!(
                        "chunk index entry missing nonce for hash {}",
                        descriptor.hash
                    )));
                }
                resolved_by_hash.insert(
                    descriptor.hash.clone(),
                    (
                        existing.telegram_file_id,
                        existing.nonce_b64,
                        descriptor.size as i64,
                    ),
                );
                continue;
            }

            nonce_by_hash.insert(descriptor.hash.clone(), descriptor.nonce_b64.clone());
            unique_descriptors.insert(descriptor.hash.clone(), descriptor);
        }

        for (hash, (_, _, size)) in &resolved_by_hash {
            let refs = parts_by_hash.get(hash).map(|v| v.len()).unwrap_or(1) as u64;
            processed_bytes += (*size as u64) * refs;
        }
        if processed_bytes > 0 {
            self.progress.transition(
                job_id,
                &file_name,
                TransferState::Running,
                TransferPhase::Uploading,
                Some(StorageMode::Chunked),
                processed_bytes,
                file_size,
                None,
            );
        }

        for descriptor in unique_descriptors.into_values() {
            let ref_count = parts_by_hash
                .get(&descriptor.hash)
                .map(|refs| refs.len())
                .unwrap_or(1) as u64;

            let permit = semaphore.clone().acquire_owned().await.map_err(|_| {
                AppError::Concurrency("failed acquiring upload semaphore".to_string())
            })?;
            let telegram = self.telegram.clone();
            let cache = self.cache.clone();
            let name_for_ext = file_name.clone();
            let hash = descriptor.hash.clone();
            let size = descriptor.size as i64;
            let progress = self.progress.clone();
            let cancel_job_id = job_id.to_string();

            uploads.spawn(async move {
                let _permit = permit;
                if progress.is_cancelled(&cancel_job_id) {
                    return Err(AppError::Validation("upload cancelled".to_string()));
                }
                cache
                    .write_chunk(&descriptor.hash, &descriptor.bytes)
                    .await?;
                let telegram_id = telegram
                    .upload_chunk(descriptor.bytes, &name_for_ext)
                    .await?;
                Ok::<(String, String, i64, u64), AppError>((hash, telegram_id, size, ref_count))
            });
        }

        while let Some(result) = uploads.join_next().await {
            if self.progress.is_cancelled(job_id) {
                uploads.abort_all();
                self.progress.transition(
                    job_id,
                    &file_name,
                    TransferState::Cancelled,
                    TransferPhase::Cancelled,
                    Some(StorageMode::Chunked),
                    processed_bytes,
                    file_size,
                    Some("upload cancelled".to_string()),
                );
                return Err(AppError::Validation("upload cancelled".to_string()));
            }
            match result {
                Ok(Ok((hash, telegram_file_id, size, ref_count))) => {
                    let nonce_b64 = nonce_by_hash.get(&hash).cloned().ok_or_else(|| {
                        AppError::Validation(format!("missing canonical nonce for hash {hash}"))
                    })?;
                    resolved_by_hash.insert(hash, (telegram_file_id, nonce_b64, size));
                    processed_bytes += (size as u64) * ref_count;
                    self.progress.transition(
                        job_id,
                        &file_name,
                        TransferState::Running,
                        TransferPhase::Uploading,
                        Some(StorageMode::Chunked),
                        processed_bytes,
                        file_size,
                        None,
                    );
                }
                Ok(Err(e)) => {
                    if e.to_string().contains("upload cancelled") {
                        uploads.abort_all();
                        self.progress.transition(
                            job_id,
                            &file_name,
                            TransferState::Cancelled,
                            TransferPhase::Cancelled,
                            Some(StorageMode::Chunked),
                            processed_bytes,
                            file_size,
                            Some(e.to_string()),
                        );
                        return Err(e);
                    }
                    error!(error = %e, "chunk upload failed");
                    self.progress.transition(
                        job_id,
                        &file_name,
                        TransferState::Failed,
                        TransferPhase::Failed,
                        Some(StorageMode::Chunked),
                        processed_bytes,
                        file_size,
                        Some(e.to_string()),
                    );
                    return Err(e);
                }
                Err(join_err) => {
                    if join_err.is_cancelled() {
                        self.progress.transition(
                            job_id,
                            &file_name,
                            TransferState::Cancelled,
                            TransferPhase::Cancelled,
                            Some(StorageMode::Chunked),
                            processed_bytes,
                            file_size,
                            Some("upload cancelled".to_string()),
                        );
                        return Err(AppError::Validation("upload cancelled".to_string()));
                    }
                    let msg = format!("upload worker join error: {join_err}");
                    self.progress.transition(
                        job_id,
                        &file_name,
                        TransferState::Failed,
                        TransferPhase::Failed,
                        Some(StorageMode::Chunked),
                        processed_bytes,
                        file_size,
                        Some(msg.clone()),
                    );
                    return Err(AppError::Concurrency(msg));
                }
            }
        }

        for (hash, part_indexes) in parts_by_hash {
            let (telegram_file_id, nonce_b64, size) =
                resolved_by_hash.get(&hash).cloned().ok_or_else(|| {
                    AppError::Validation(format!("missing resolved chunk mapping for hash {hash}"))
                })?;
            for part_index in part_indexes {
                uploaded.push(NewChunkRecord {
                    part_index,
                    hash: hash.clone(),
                    telegram_file_id: telegram_file_id.clone(),
                    size,
                    nonce_b64: nonce_b64.clone(),
                });
            }
        }

        uploaded.sort_by_key(|c| c.part_index);
        let safe_name = self.db.resolve_conflict_name(folder_id, &file_name)?;
        let file_id = self.db.persist_uploaded_file(
            NewFileRecord {
                name: safe_name,
                size: file_size as i64,
                hash: file_hash,
                folder_id,
                mime_type: classify_mime(file_path.as_path()),
                original_path: Some(file_path.to_string_lossy().to_string()),
                storage_mode: StorageMode::Chunked,
                telegram_file_id: None,
            },
            uploaded,
        )?;

        self.progress.transition(
            job_id,
            &file_name,
            TransferState::Completed,
            TransferPhase::Completed,
            Some(StorageMode::Chunked),
            file_size,
            file_size,
            None,
        );
        Ok(file_id)
    }
}

fn compute_parallelism(max_parallelism: usize) -> usize {
    let cpu = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let adaptive = (cpu * 3).clamp(4, 48);
    adaptive.min(max_parallelism.max(1))
}

fn storage_mode_for_size(size: u64) -> StorageMode {
    if size <= SINGLE_UPLOAD_THRESHOLD_BYTES {
        StorageMode::Single
    } else {
        StorageMode::Chunked
    }
}
