use crate::cache::LocalCdnCache;
use crate::chunking::ChunkingEngine;
use crate::database::Database;
use crate::models::{
    AppError, AppResult, CachePersistenceState, DownloadCacheEvent, DownloadCacheMode,
    DownloadResponse, FileOrigin, PreviewResponse, SettingsDto, StorageMode, TransferPhase,
    TransferState,
};
use crate::progress::ProgressHub;
use crate::telegram::TelegramClient;
use futures::stream::{FuturesUnordered, StreamExt};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
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
        settings: SettingsDto,
        requested_cache_mode: DownloadCacheMode,
        app_handle: Option<AppHandle>,
    ) -> AppResult<DownloadResponse> {
        let file = self.db.get_file(file_id)?;
        let job_id = format!("download-{}", Uuid::new_v4());
        let total = file.size.max(0) as u64;
        let effective_cache_mode =
            settings.resolve_download_cache_mode(total, requested_cache_mode);

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
            cache_mode = ?effective_cache_mode,
            bytes_total = total
        );

        async move {
            match file.storage_mode {
                StorageMode::Single => {
                    self.download_single_file(
                        &job_id,
                        &file.name,
                        &file.hash,
                        &file.origin,
                        file.telegram_file_id,
                        destination_path,
                        total,
                        effective_cache_mode,
                        app_handle,
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
                        effective_cache_mode,
                        app_handle,
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
        file_origin: &FileOrigin,
        telegram_file_id: Option<String>,
        destination_path: PathBuf,
        total: u64,
        cache_mode: DownloadCacheMode,
        app_handle: Option<AppHandle>,
    ) -> AppResult<DownloadResponse> {
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
            return Ok(DownloadResponse {
                cache_state: CachePersistenceState::Completed,
                cache_mode,
                message: Some("Download concluído com cache local.".to_string()),
            });
        }

        let telegram_file_id = telegram_file_id.ok_or_else(|| {
            AppError::Validation("single-object file missing telegram_file_id".to_string())
        })?;
        let temp_path = destination_path.with_extension("download.tmp");
        self.progress.wait_if_paused(job_id).await;
        if self.progress.is_cancelled(job_id) {
            self.progress.transition(
                job_id,
                file_name,
                TransferState::Cancelled,
                TransferPhase::Cancelled,
                Some(StorageMode::Single),
                0,
                total,
                None,
            );
            return Err(AppError::Validation("download cancelled".to_string()));
        }

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
        if should_validate_single_object_hash(file_hash, file_origin) && digest != file_hash {
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

        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::rename(&temp_path, &destination_path).await?;

        let response = match cache_mode {
            DownloadCacheMode::Enabled => {
                let cache = self.cache.clone();
                let file_name_owned = file_name.to_string();
                let source_path = destination_path.clone();
                let hash = file_hash.to_string();
                emit_cache_event(
                    &app_handle,
                    &file_name_owned,
                    CachePersistenceState::Pending,
                    Some("Cache agendado em background.".to_string()),
                );
                tokio::spawn(async move {
                    emit_cache_event(
                        &app_handle,
                        &file_name_owned,
                        CachePersistenceState::Writing,
                        Some("Persistindo cache em background.".to_string()),
                    );
                    match cache.import_file(&hash, &source_path).await {
                        Ok(()) => emit_cache_event(
                            &app_handle,
                            &file_name_owned,
                            CachePersistenceState::Completed,
                            Some("Cache concluído.".to_string()),
                        ),
                        Err(e) => {
                            error!(file = %file_name_owned, error = %e, "single-file cache write failed");
                            emit_cache_event(
                                &app_handle,
                                &file_name_owned,
                                CachePersistenceState::Failed,
                                Some(format!("Falha ao persistir cache: {e}")),
                            );
                        }
                    }
                });
                DownloadResponse {
                    cache_state: CachePersistenceState::Pending,
                    cache_mode,
                    message: Some(
                        "Download concluído. Cache preenchendo em background.".to_string(),
                    ),
                }
            }
            DownloadCacheMode::Disabled | DownloadCacheMode::Default => DownloadResponse {
                cache_state: CachePersistenceState::Skipped,
                cache_mode,
                message: Some("Download concluído sem preencher cache.".to_string()),
            },
        };

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
        Ok(response)
    }

    async fn download_chunked_file(
        &self,
        job_id: &str,
        file_id: i64,
        file_name: &str,
        destination_path: PathBuf,
        total: u64,
        max_parallelism: usize,
        cache_mode: DownloadCacheMode,
        app_handle: Option<AppHandle>,
    ) -> AppResult<DownloadResponse> {
        let chunks = self.db.get_chunks_for_file(file_id)?;
        let semaphore = Arc::new(Semaphore::new(compute_parallelism(max_parallelism)));
        let mut futures = FuturesUnordered::new();
        let cache_write_semaphore = Arc::new(Semaphore::new(1));
        let mut cache_tasks = Vec::new();
        let mut cache_scheduled = false;
        let mut done = 0u64;

        for (part_index, hash, telegram_file_id, _size, nonce_b64) in chunks {
            self.progress.wait_if_paused(job_id).await;
            if self.progress.is_cancelled(job_id) {
                self.progress.transition(
                    job_id,
                    file_name,
                    TransferState::Cancelled,
                    TransferPhase::Cancelled,
                    Some(StorageMode::Chunked),
                    done,
                    total,
                    None,
                );
                return Err(AppError::Validation("download cancelled".to_string()));
            }
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
                    return Ok::<(i64, String, String, Vec<u8>, bool), AppError>((
                        part_index, hash, nonce_b64, bytes, true,
                    ));
                }
                let bytes = tg.download_chunk(&telegram_file_id).await?;
                Ok((part_index, hash, nonce_b64, bytes, false))
            }));
        }

        let mut ordered = BTreeMap::<i64, Vec<u8>>::new();

        while let Some(next) = futures.next().await {
            self.progress.wait_if_paused(job_id).await;
            if self.progress.is_cancelled(job_id) {
                self.progress.transition(
                    job_id,
                    file_name,
                    TransferState::Cancelled,
                    TransferPhase::Cancelled,
                    Some(StorageMode::Chunked),
                    done,
                    total,
                    None,
                );
                return Err(AppError::Validation("download cancelled".to_string()));
            }
            match next {
                Ok(Ok((part_index, hash, nonce_b64, encrypted, from_cache))) => {
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
                    if cache_mode == DownloadCacheMode::Enabled && !from_cache {
                        if !cache_scheduled {
                            cache_scheduled = true;
                            emit_cache_event(
                                &app_handle,
                                file_name,
                                CachePersistenceState::Pending,
                                Some("Cache agendado em background.".to_string()),
                            );
                        }
                        let cache = self.cache.clone();
                        let hash_for_cache = hash.clone();
                        let writer_gate = cache_write_semaphore.clone();
                        cache_tasks.push(tokio::spawn(async move {
                            let _writer = writer_gate.acquire_owned().await.map_err(|_| {
                                AppError::Concurrency("cache writer semaphore closed".to_string())
                            })?;
                            cache.write_chunk(&hash_for_cache, &encrypted).await
                        }));
                    }
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

        let response = if cache_scheduled {
            let file_name_owned = file_name.to_string();
            tokio::spawn(async move {
                emit_cache_event(
                    &app_handle,
                    &file_name_owned,
                    CachePersistenceState::Writing,
                    Some("Persistindo cache em background.".to_string()),
                );
                let mut cache_failed = None;
                for task in cache_tasks {
                    match task.await {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => {
                            cache_failed = Some(format!("Falha ao persistir cache: {e}"));
                            break;
                        }
                        Err(join_err) => {
                            cache_failed =
                                Some(format!("Falha ao aguardar cache em background: {join_err}"));
                            break;
                        }
                    }
                }
                match cache_failed {
                    Some(message) => {
                        error!(file = %file_name_owned, %message, "chunked cache write failed");
                        emit_cache_event(
                            &app_handle,
                            &file_name_owned,
                            CachePersistenceState::Failed,
                            Some(message),
                        );
                    }
                    None => emit_cache_event(
                        &app_handle,
                        &file_name_owned,
                        CachePersistenceState::Completed,
                        Some("Cache concluído.".to_string()),
                    ),
                }
            });
            DownloadResponse {
                cache_state: CachePersistenceState::Pending,
                cache_mode,
                message: Some("Download concluído. Cache preenchendo em background.".to_string()),
            }
        } else if cache_mode == DownloadCacheMode::Enabled {
            DownloadResponse {
                cache_state: CachePersistenceState::Completed,
                cache_mode,
                message: Some("Download concluído com cache local.".to_string()),
            }
        } else {
            DownloadResponse {
                cache_state: CachePersistenceState::Skipped,
                cache_mode,
                message: Some("Download concluído sem preencher cache.".to_string()),
            }
        };

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
        Ok(response)
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
        let _ = self
            .download_file(
                file_id,
                temp.clone(),
                4,
                SettingsDto::default(),
                DownloadCacheMode::Disabled,
                None,
            )
            .await?;

        Ok(PreviewResponse {
            local_path: temp.to_string_lossy().to_string(),
            mime_type: file.mime_type,
        })
    }
}

fn should_validate_single_object_hash(file_hash: &str, file_origin: &FileOrigin) -> bool {
    if matches!(file_origin, FileOrigin::Imported) {
        return is_sha256_hex(file_hash);
    }
    is_sha256_hex(file_hash)
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::{is_sha256_hex, should_validate_single_object_hash};
    use crate::models::FileOrigin;

    #[test]
    fn validates_savedrive_single_files_with_real_sha256() {
        let hash = "666b61e0d0b1baf4bb0f6b2160e20085c0371fa572368a2a5f28448354b7b18b";
        assert!(is_sha256_hex(hash));
        assert!(should_validate_single_object_hash(hash, &FileOrigin::Savedrive));
    }

    #[test]
    fn skips_strict_hash_validation_for_imported_telegram_placeholders() {
        let placeholder = "telegram-import:127809";
        assert!(!is_sha256_hex(placeholder));
        assert!(!should_validate_single_object_hash(
            placeholder,
            &FileOrigin::Imported
        ));
    }
}

fn emit_cache_event(
    app_handle: &Option<AppHandle>,
    file_name: &str,
    state: CachePersistenceState,
    message: Option<String>,
) {
    if let Some(app) = app_handle {
        let payload = DownloadCacheEvent {
            file_name: file_name.to_string(),
            state,
            message,
        };
        let _ = app.emit("download_cache_state_changed", payload);
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
