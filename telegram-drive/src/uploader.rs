use crate::cache::LocalCdnCache;
use crate::chunking::{ChunkPipelineProgress, ChunkingEngine};
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
        let job_id = format!("upload-{}", Uuid::new_v4());
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

        self.progress.wait_if_paused(job_id).await;
        if self.progress.is_cancelled(job_id) {
            self.progress.transition(
                job_id,
                &file_name,
                TransferState::Cancelled,
                TransferPhase::Cancelled,
                Some(StorageMode::Single),
                0,
                file_size,
                None,
            );
            return Err(AppError::Validation("upload cancelled".to_string()));
        }

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

        // ── Pipeline de streaming com progresso em tempo real ──
        //
        // Arquitetura:
        //   Tarefa A (background): lê disco → criptografa → envia pelo chunk_rx
        //   Loop principal (aqui): recebe chunk → decide dedup → spawna upload
        //   Tarefa de coleta: coleta resultados de uploads e atualiza progresso
        //
        // O progresso é emitido em DOIS momentos:
        //   1. Quando um chunk é LIDO (fase Hashing/Preparing) — imediato
        //   2. Quando um chunk é ENVIADO ao Telegram — confirma bytes transferidos
        //
        // Isso evita que a UI fique travada durante leituras longas de disco.
        let (meta_rx, mut progress_rx, mut chunk_rx) =
            chunking.stream_and_encrypt_chunks(file_path.clone(), file_size);

        let adaptive = compute_parallelism(max_parallelism);
        let semaphore = Arc::new(Semaphore::new(adaptive));
        let mut uploads: JoinSet<AppResult<(String, String, i64, u64)>> = JoinSet::new();

        let mut parts_by_hash: HashMap<String, Vec<i64>> = HashMap::new();
        let mut resolved_by_hash: HashMap<String, (String, String, i64)> = HashMap::new();
        let mut nonce_by_hash: HashMap<String, String> = HashMap::new();
        let mut uploaded: Vec<NewChunkRecord> = Vec::new();
        let mut bytes_read: u64 = 0;
        let mut encrypted_bytes: u64 = 0;
        let mut processed_bytes: u64 = 0;
        let mut progress_stream_closed = false;
        let mut chunk_stream_closed = false;

        loop {
            if progress_stream_closed && chunk_stream_closed && uploads.is_empty() {
                break;
            }

            // Usa tokio::select! para consumir chunks, telemetria do pipeline E coletar resultados
            // de upload simultaneamente, evitando que um bloqueie o outro.
            tokio::select! {
                progress_event = progress_rx.recv(), if !progress_stream_closed => {
                    match progress_event {
                        Some(ChunkPipelineProgress { phase: TransferPhase::Chunking, bytes_done, bytes_total }) => {
                            bytes_read = bytes_done;
                            self.progress.transition(
                                job_id, &file_name,
                                TransferState::Running, TransferPhase::Chunking,
                                Some(StorageMode::Chunked), bytes_done, bytes_total, None,
                            );
                        }
                        Some(ChunkPipelineProgress { phase: TransferPhase::Encrypting, bytes_done, bytes_total }) => {
                            encrypted_bytes = bytes_done;
                            self.progress.transition(
                                job_id, &file_name,
                                TransferState::Running, TransferPhase::Encrypting,
                                Some(StorageMode::Chunked), bytes_done, bytes_total, None,
                            );
                        }
                        Some(ChunkPipelineProgress { phase, bytes_done, bytes_total }) => {
                            self.progress.transition(
                                job_id, &file_name,
                                TransferState::Running, phase,
                                Some(StorageMode::Chunked), bytes_done, bytes_total, None,
                            );
                        }
                        None => {
                            progress_stream_closed = true;
                        }
                    }
                }

                // Ramo 1: novo chunk pronto para upload
                chunk_result = chunk_rx.recv(), if !chunk_stream_closed => {
                    self.progress.wait_if_paused(job_id).await;
                    let Some(chunk_result) = chunk_result else {
                        chunk_stream_closed = true;
                        continue;
                    };

                    if self.progress.is_cancelled(job_id) {
                        drop(chunk_rx);
                        self.progress.transition(
                            job_id, &file_name,
                            TransferState::Cancelled, TransferPhase::Cancelled,
                            Some(StorageMode::Chunked), processed_bytes, file_size, None,
                        );
                        return Err(AppError::Validation("upload cancelled".to_string()));
                    }

                    let descriptor = chunk_result.map_err(|e| {
                        self.progress.transition(
                            job_id, &file_name,
                            TransferState::Failed, TransferPhase::Failed,
                            Some(StorageMode::Chunked), processed_bytes, file_size,
                            Some(e.to_string()),
                        );
                        e
                    })?;

                    // Neste ponto o chunk j? foi lido e criptografado; entra na fase de upload efetivo.
                    self.progress.transition(
                        job_id, &file_name,
                        TransferState::Running, TransferPhase::Uploading,
                        Some(StorageMode::Chunked), encrypted_bytes.max(bytes_read), file_size, None,
                    );

                    parts_by_hash
                        .entry(descriptor.hash.clone())
                        .or_default()
                        .push(descriptor.part_index);

                    // Dedup: chunk j? resolvido neste job
                    if resolved_by_hash.contains_key(&descriptor.hash) {
                        processed_bytes += descriptor.size as u64;
                        continue;
                    }

                    // Dedup: chunk j? existe no Telegram
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
                            (existing.telegram_file_id, existing.nonce_b64, descriptor.size as i64),
                        );
                        processed_bytes += descriptor.size as u64;
                        continue;
                    }

                    nonce_by_hash.insert(descriptor.hash.clone(), descriptor.nonce_b64.clone());

                    // Spawn upload ? acquire_owned() n?o bloqueia aqui pois o
                    // select! alterna entre ramos, ent?o uploads em andamento
                    // continuam progredindo enquanto esperamos o sem?foro.
                    let permit = semaphore.clone().acquire_owned().await.map_err(|_| {
                        AppError::Concurrency("failed acquiring upload semaphore".to_string())
                    })?;
                    let telegram = self.telegram.clone();
                    let cache = self.cache.clone();
                    let name_for_ext = file_name.clone();
                    let hash = descriptor.hash.clone();
                    let size = descriptor.size as i64;
                    let chunk_bytes_count = descriptor.size as u64;
                    let progress_clone = self.progress.clone();
                    let cancel_job_id = job_id.to_string();

                    uploads.spawn(async move {
                        let _permit = permit;
                        progress_clone.wait_if_paused(&cancel_job_id).await;
                        if progress_clone.is_cancelled(&cancel_job_id) {
                            return Err(AppError::Validation("upload cancelled".to_string()));
                        }
                        cache.write_chunk(&descriptor.hash, &descriptor.bytes).await?;
                        let telegram_id = telegram
                            .upload_chunk(descriptor.bytes, &name_for_ext)
                            .await?;
                        Ok((hash, telegram_id, size, chunk_bytes_count))
                    });
                }

                // Ramo 2: um upload terminou — coleta o resultado sem bloquear leitura
                Some(result) = uploads.join_next(), if !uploads.is_empty() => {
                    self.progress.wait_if_paused(job_id).await;
                    match result {
                        Ok(Ok((hash, telegram_id, size, bytes_count))) => {
                            let nonce = nonce_by_hash.get(&hash).cloned().unwrap_or_default();
                            resolved_by_hash.insert(hash.clone(), (telegram_id, nonce, size));
                            processed_bytes += bytes_count;
                            // Progresso de upload confirmado — mostra bytes reais enviados
                            self.progress.transition(
                                job_id, &file_name,
                                TransferState::Running, TransferPhase::Uploading,
                                Some(StorageMode::Chunked),
                                processed_bytes.max(bytes_read), file_size, None,
                            );
                        }
                        Ok(Err(e)) => {
                            let is_cancel = e.to_string().contains("upload cancelled");
                            self.progress.transition(
                                job_id, &file_name,
                                if is_cancel { TransferState::Cancelled } else { TransferState::Failed },
                                if is_cancel { TransferPhase::Cancelled } else { TransferPhase::Failed },
                                Some(StorageMode::Chunked), processed_bytes, file_size,
                                Some(e.to_string()),
                            );
                            return Err(e);
                        }
                        Err(join_err) => {
                            let e = AppError::Concurrency(join_err.to_string());
                            self.progress.transition(
                                job_id, &file_name,
                                TransferState::Failed, TransferPhase::Failed,
                                Some(StorageMode::Chunked), processed_bytes, file_size,
                                Some(e.to_string()),
                            );
                            return Err(e);
                        }
                    }
                }
            }
        }

        // Aguarda todos os uploads pendentes terminarem
        while let Some(result) = uploads.join_next().await {
            match result {
                Ok(Ok((hash, telegram_id, size, bytes_count))) => {
                    let nonce = nonce_by_hash.get(&hash).cloned().unwrap_or_default();
                    resolved_by_hash.insert(hash.clone(), (telegram_id, nonce, size));
                    processed_bytes += bytes_count;
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
                    error!(error = %e, "chunk upload failed");
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
                        processed_bytes,
                        file_size,
                        Some(e.to_string()),
                    );
                    return Err(e);
                }
                Err(join_err) => {
                    let e = AppError::Concurrency(join_err.to_string());
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
            }
        }

        // Recupera hash e tamanho real do arquivo produzidos pelo pipeline
        let (file_hash, actual_size) = meta_rx
            .await
            .map_err(|_| {
                AppError::Concurrency("chunking pipeline task dropped unexpectedly".to_string())
            })?
            .map_err(|e| {
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
                e
            })?;

        // Dedup no nível do arquivo (após ter o hash completo)
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
                actual_size,
                actual_size,
                None,
            );
            info!(file = %file_name, "file-level dedup hit after pipeline; created reference");
            return Ok(new_id);
        }

        // Monta lista final de chunks na ordem correta
        for (hash, part_indexes) in &parts_by_hash {
            let (telegram_file_id, nonce_b64, size) =
                resolved_by_hash.get(hash).ok_or_else(|| {
                    AppError::Validation(format!("missing resolved chunk mapping for hash {hash}"))
                })?;
            for part_index in part_indexes {
                uploaded.push(NewChunkRecord {
                    part_index: *part_index,
                    hash: hash.clone(),
                    telegram_file_id: telegram_file_id.clone(),
                    size: *size,
                    nonce_b64: nonce_b64.clone(),
                });
            }
        }

        uploaded.sort_by_key(|c| c.part_index);
        let safe_name = self.db.resolve_conflict_name(folder_id, &file_name)?;
        let file_id = self.db.persist_uploaded_file(
            NewFileRecord {
                name: safe_name,
                size: actual_size as i64,
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
            actual_size,
            actual_size,
            None,
        );
        Ok(file_id)
    }
}

/// Tenta coletar resultados prontos do JoinSet sem bloquear.
/// Sem tokio_unstable, retorna None — os resultados são coletados
/// no loop final de join_next ao fim do pipeline. Correto e seguro.
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
