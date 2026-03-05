use crate::cache::LocalCdnCache;
use crate::chunking::ChunkingEngine;
use crate::database::Database;
use crate::models::{
    AppError, AppResult, PreviewResponse, StorageMode, TransferPhase, TransferState,
};
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
use tracing::{error, info_span, Instrument};
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

    pub async fn download_file(
        &self,
        file_id: i64,
        destination_path: PathBuf,
        max_parallelism: usize,
    ) -> AppResult<()> {
        let file = self.db.get_file(file_id)?;
        let job_id = Uuid::new_v4().to_string();
        let total = file.size.max(0) as u64;

        self.progress.transition(
            &job_id,
            &file.name,
            TransferState::Running,
            TransferPhase::Downloading,
            Some(file.storage_mode.clone()),
            0,
            total,
            None,
        );

        let span = info_span!(
            "download_file",
            job_id = %job_id,
            file_id,
            storage_mode = ?file.storage_mode,
            bytes_total = total
        );

        async move {
            match file.storage_mode {
                StorageMode::Single => {
                    self.download_single_file(
                        &job_id,
                        &file.name,
                        &file.hash,
                        file.telegram_file_id,
                        destination_path,
                        total,
                    )
                    .await
                }
                StorageMode::Chunked => {
                    self.download_chunked_file(
                        &job_id,
                        file_id,
                        &file.name,
                        destination_path,
                        total,
                        max_parallelism,
                    )
                    .await
                }
            }
        }
        .instrument(span)
        .await
    }

    async fn download_single_file(
        &self,
        job_id: &str,
        file_name: &str,
        file_hash: &str,
        telegram_file_id: Option<String>,
        destination_path: PathBuf,
        total: u64,
    ) -> AppResult<()> {
        if self.cache.copy_to(file_hash, &destination_path).await? {
            self.progress.transition(
                job_id,
                file_name,
                TransferState::Completed,
                TransferPhase::Completed,
                Some(StorageMode::Single),
                total,
                total,
                None,
            );
            return Ok(());
        }

        let telegram_file_id = telegram_file_id.ok_or_else(|| {
            AppError::Validation("single-object file missing telegram_file_id".to_string())
        })?;
        let temp_path = destination_path.with_extension("download.tmp");

        self.telegram
            .download_file_to_path(&telegram_file_id, &temp_path)
            .await
            .map_err(|e| {
                self.progress.transition(
                    job_id,
                    file_name,
                    TransferState::Failed,
                    TransferPhase::Failed,
                    Some(StorageMode::Single),
                    0,
                    total,
                    Some(e.to_string()),
                );
                e
            })?;

        let bytes = fs::read(&temp_path).await?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let digest = hex::encode(hasher.finalize());
        if digest != file_hash {
            let _ = fs::remove_file(&temp_path).await;
            let err = AppError::Validation(format!(
                "single-object hash mismatch expected={} got={}",
                file_hash, digest
            ));
            self.progress.transition(
                job_id,
                file_name,
                TransferState::Failed,
                TransferPhase::Failed,
                Some(StorageMode::Single),
                0,
                total,
                Some(err.to_string()),
            );
            return Err(err);
        }

        self.cache.import_file(file_hash, &temp_path).await?;
        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::rename(&temp_path, &destination_path).await?;

        self.progress.transition(
            job_id,
            file_name,
            TransferState::Completed,
            TransferPhase::Completed,
            Some(StorageMode::Single),
            total,
            total,
            None,
        );
        Ok(())
    }

    async fn download_chunked_file(
        &self,
        job_id: &str,
        file_id: i64,
        file_name: &str,
        destination_path: PathBuf,
        total: u64,
        max_parallelism: usize,
    ) -> AppResult<()> {
        let chunks = self.db.get_chunks_for_file(file_id)?;
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
                    return Ok::<(i64, String, String, Vec<u8>), AppError>((
                        part_index, hash, nonce_b64, bytes,
                    ));
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
                        let err = AppError::Validation(format!(
                            "hash mismatch in part {} expected={} got={}",
                            part_index, hash, digest
                        ));
                        self.progress.transition(
                            job_id,
                            file_name,
                            TransferState::Failed,
                            TransferPhase::Failed,
                            Some(StorageMode::Chunked),
                            done,
                            total,
                            Some(err.to_string()),
                        );
                        return Err(err);
                    }
                    done += decrypted.len() as u64;
                    ordered.insert(part_index, decrypted);
                    self.progress.transition(
                        job_id,
                        file_name,
                        TransferState::Running,
                        TransferPhase::Downloading,
                        Some(StorageMode::Chunked),
                        done,
                        total,
                        None,
                    );
                }
                Ok(Err(e)) => {
                    self.progress.transition(
                        job_id,
                        file_name,
                        TransferState::Failed,
                        TransferPhase::Failed,
                        Some(StorageMode::Chunked),
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
                        job_id,
                        file_name,
                        TransferState::Failed,
                        TransferPhase::Failed,
                        Some(StorageMode::Chunked),
                        done,
                        total,
                        Some(msg.clone()),
                    );
                    return Err(AppError::Concurrency(msg));
                }
            }
        }

        self.progress.transition(
            job_id,
            file_name,
            TransferState::Running,
            TransferPhase::Reassembling,
            Some(StorageMode::Chunked),
            total,
            total,
            None,
        );

        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let partial = destination_path.with_extension("partial");
        write_reassembled_file(&partial, &ordered).await?;
        fs::rename(&partial, &destination_path).await?;

        self.progress.transition(
            job_id,
            file_name,
            TransferState::Completed,
            TransferPhase::Completed,
            Some(StorageMode::Chunked),
            total,
            total,
            None,
        );
        Ok(())
    }

    pub async fn materialize_preview(&self, file_id: i64) -> AppResult<PreviewResponse> {
        let file = self.db.get_file(file_id)?;
        if !file.mime_type.starts_with("image/") {
            return Err(AppError::Validation(
                "preview is only available for image files".to_string(),
            ));
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
