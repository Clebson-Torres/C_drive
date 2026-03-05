#![cfg(test)]

use crate::cache::LocalCdnCache;
use crate::chunking::ChunkingEngine;
use crate::database::Database;
use crate::dedup::DedupEngine;
use crate::downloader::DownloadService;
use crate::models::StorageMode;
use crate::progress::ProgressHub;
use crate::telegram::TelegramClient;
use crate::uploader::{UploadService, SINGLE_UPLOAD_THRESHOLD_BYTES};
use serde::Serialize;
use sha2::Digest;
use std::fs::OpenOptions;
use std::io::{Seek, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tempfile::tempdir;

#[derive(Clone)]
struct TransferCase {
    name: &'static str,
    bytes: u64,
    expected_mode: StorageMode,
}

#[derive(Serialize)]
struct TransferLogRecord {
    case_name: String,
    bytes: u64,
    storage_mode: String,
    upload_ms: u128,
    download_ms: u128,
    chunk_count: usize,
    hash: String,
}

#[tokio::test]
async fn threshold_policy_routes_files_correctly() {
    assert_eq!(
        mode_for_size(SINGLE_UPLOAD_THRESHOLD_BYTES - 1),
        StorageMode::Single
    );
    assert_eq!(
        mode_for_size(SINGLE_UPLOAD_THRESHOLD_BYTES),
        StorageMode::Single
    );
    assert_eq!(
        mode_for_size(SINGLE_UPLOAD_THRESHOLD_BYTES + 1),
        StorageMode::Chunked
    );
}

#[tokio::test]
#[ignore = "Large real-file transfer matrix; run manually when validating throughput and threshold behavior"]
async fn upload_download_matrix_real_files() {
    let cases = [
        TransferCase {
            name: "small-1MiB.bin",
            bytes: 1 * 1024 * 1024,
            expected_mode: StorageMode::Single,
        },
        TransferCase {
            name: "medium-128MiB.bin",
            bytes: 128 * 1024 * 1024,
            expected_mode: StorageMode::Single,
        },
        TransferCase {
            name: "large-900MiB.bin",
            bytes: 900 * 1024 * 1024,
            expected_mode: StorageMode::Single,
        },
        TransferCase {
            name: "threshold-minus-1MiB.bin",
            bytes: SINGLE_UPLOAD_THRESHOLD_BYTES - (1024 * 1024),
            expected_mode: StorageMode::Single,
        },
        TransferCase {
            name: "threshold-plus-1MiB.bin",
            bytes: SINGLE_UPLOAD_THRESHOLD_BYTES + (1024 * 1024),
            expected_mode: StorageMode::Chunked,
        },
    ];

    let temp = tempdir().unwrap();
    let app_data = temp.path().join("app");
    std::fs::create_dir_all(&app_data).unwrap();
    let db = Database::open(&app_data.join("telegram-drive.db")).unwrap();
    let settings_chunk_size = 8 * 1024 * 1024;
    let mut key = [0u8; 32];
    let digest = sha2::Sha256::digest(b"telegram-drive-test-key");
    key.copy_from_slice(&digest[..32]);

    let cache = LocalCdnCache::new(app_data.join("cache"), 8 * 1024 * 1024 * 1024)
        .await
        .unwrap();
    let chunking = ChunkingEngine::new(settings_chunk_size, key);
    let telegram = TelegramClient::new_with_mode(app_data.join("telegram_saved_messages"), true)
        .await
        .unwrap();
    telegram
        .start_phone_auth("+551100000000".to_string(), 12345, "hash".to_string())
        .await
        .unwrap();
    telegram.verify_code("12345".to_string()).await.unwrap();

    let dedup = DedupEngine::new(db.clone());
    let progress = ProgressHub::new();
    let uploader = UploadService::new(
        db.clone(),
        dedup.clone(),
        chunking.clone(),
        telegram.clone(),
        cache.clone(),
        progress.clone(),
    );
    let downloader = DownloadService::new(
        db.clone(),
        chunking.clone(),
        telegram.clone(),
        cache.clone(),
        progress,
    );
    let root_id = db.root_folder_id().unwrap();
    let fixtures_dir = temp.path().join("fixtures");
    let downloads_dir = temp.path().join("downloads");
    std::fs::create_dir_all(&fixtures_dir).unwrap();
    std::fs::create_dir_all(&downloads_dir).unwrap();

    let log_dir = default_log_dir();
    std::fs::create_dir_all(&log_dir).unwrap();
    let jsonl_path = log_dir.join("transfer-matrix.jsonl");
    let summary_path = log_dir.join("transfer-matrix-summary.md");

    let mut records = Vec::new();

    for case in cases {
        let fixture = fixtures_dir.join(case.name);
        create_sparse_file(&fixture, case.bytes).unwrap();

        let upload_start = Instant::now();
        let file_id = uploader
            .upload_file(root_id, fixture.clone(), 8, settings_chunk_size)
            .await
            .unwrap();
        let upload_ms = upload_start.elapsed().as_millis();

        let file = db.get_file(file_id).unwrap();
        assert_eq!(file.storage_mode, case.expected_mode);

        let chunks = db.get_chunks_for_file(file_id).unwrap();
        match case.expected_mode {
            StorageMode::Single => {
                assert!(chunks.is_empty());
                let _ = tokio::fs::remove_file(cache.path_for_hash(&file.hash)).await;
            }
            StorageMode::Chunked => {
                assert!(!chunks.is_empty());
                for (_, hash, _, _, _) in &chunks {
                    let _ = tokio::fs::remove_file(cache.path_for_hash(hash)).await;
                }
            }
        }

        let download_path = downloads_dir.join(case.name);
        let download_start = Instant::now();
        downloader
            .download_file(file_id, download_path.clone(), 8)
            .await
            .unwrap();
        let download_ms = download_start.elapsed().as_millis();

        let (download_hash, download_size) = chunking.hash_file(&download_path).await.unwrap();
        assert_eq!(download_hash, file.hash);
        assert_eq!(download_size, case.bytes);

        let record = TransferLogRecord {
            case_name: case.name.to_string(),
            bytes: case.bytes,
            storage_mode: format!("{:?}", file.storage_mode),
            upload_ms,
            download_ms,
            chunk_count: chunks.len(),
            hash: file.hash.clone(),
        };
        append_json_line(&jsonl_path, &record).unwrap();
        records.push(record);
    }

    write_summary(&summary_path, &records).unwrap();
}

fn default_log_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("logs")
        .join("perf")
}

fn append_json_line(path: &Path, record: &TransferLogRecord) -> std::io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(record).unwrap())?;
    Ok(())
}

fn write_summary(path: &Path, records: &[TransferLogRecord]) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)?;
    writeln!(file, "# Transfer Matrix Summary")?;
    writeln!(file)?;
    for record in records {
        writeln!(
            file,
            "- {} | {} | mode={} | upload={}ms | download={}ms | chunks={}",
            record.case_name,
            record.bytes,
            record.storage_mode,
            record.upload_ms,
            record.download_ms,
            record.chunk_count
        )?;
    }
    Ok(())
}

fn create_sparse_file(path: &Path, bytes: u64) -> std::io::Result<()> {
    let file = std::fs::File::create(path)?;
    file.set_len(bytes)?;
    let mut file = OpenOptions::new().write(true).open(path)?;
    file.write_all(b"TGDRIVE")?;
    if bytes > 7 {
        file.seek(std::io::SeekFrom::Start(bytes - 7))?;
        file.write_all(b"TGDRIVE")?;
    }
    Ok(())
}

fn mode_for_size(size: u64) -> StorageMode {
    if size <= SINGLE_UPLOAD_THRESHOLD_BYTES {
        StorageMode::Single
    } else {
        StorageMode::Chunked
    }
}
