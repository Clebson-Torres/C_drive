#![cfg(test)]

use crate::auth::AuthService;
use crate::cache::LocalCdnCache;
use crate::chunking::ChunkingEngine;
use crate::database::Database;
use crate::dedup::DedupEngine;
use crate::downloader::DownloadService;
use crate::models::StorageMode;
use crate::models::{DownloadCacheMode, SettingsDto};
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
use tokio::fs::File;
use tokio::io::AsyncReadExt;

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

#[derive(Serialize)]
struct ChunkBenchmarkRecord {
    chunk_size_bytes: usize,
    bytes: u64,
    prep_ms: u128,
    mib_per_sec: f64,
    chunk_count: usize,
}

#[derive(Serialize)]
struct RealUploadCompareRecord {
    chunk_size_bytes: usize,
    bytes: u64,
    upload_ms: u128,
    mib_per_sec: f64,
    chunk_count: usize,
    storage_mode: String,
    file_name: String,
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
            .download_file(
                file_id,
                download_path.clone(),
                8,
                SettingsDto::default(),
                DownloadCacheMode::Default,
                None,
            )
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

#[tokio::test]
#[ignore = "Manual large-file benchmark for chunk size comparison"]
async fn benchmark_chunk_sizes_for_large_uploads() {
    let fixture_bytes = benchmark_fixture_bytes();
    let chunk_sizes = [
        crate::models::CHUNK_SIZE_64_MIB,
        crate::models::CHUNK_SIZE_128_MIB,
        crate::models::CHUNK_SIZE_256_MIB,
    ];

    let log_dir = default_log_dir();
    std::fs::create_dir_all(&log_dir).unwrap();
    let jsonl_path = log_dir.join("chunk-benchmark.jsonl");
    let summary_path = log_dir.join("chunk-benchmark-summary.md");
    let _ = std::fs::remove_file(&jsonl_path);
    let _ = std::fs::remove_file(&summary_path);
    let mut records = Vec::new();

    for chunk_size in chunk_sizes {
        let temp = tempdir().unwrap();
        let mut key = [0u8; 32];
        let digest = sha2::Sha256::digest(b"telegram-drive-benchmark-key");
        key.copy_from_slice(&digest[..32]);

        let chunking = ChunkingEngine::new(chunk_size, key);
        let fixture = temp
            .path()
            .join(format!("benchmark-{}MiB.bin", chunk_size / 1024 / 1024));
        create_sparse_file(&fixture, fixture_bytes).unwrap();

        let prep_start = Instant::now();
        let (total_bytes, chunk_count) = benchmark_chunk_preparation(&chunking, &fixture).await;
        let prep_ms = prep_start.elapsed().as_millis();
        let mib_per_sec =
            total_bytes as f64 / 1024_f64 / 1024_f64 / (prep_ms.max(1) as f64 / 1000.0);

        let record = ChunkBenchmarkRecord {
            chunk_size_bytes: chunk_size,
            bytes: total_bytes,
            prep_ms,
            mib_per_sec,
            chunk_count,
        };
        append_json_line(&jsonl_path, &record).unwrap();
        records.push(record);
    }

    write_chunk_benchmark_summary(&summary_path, &records).unwrap();
}

#[tokio::test]
#[ignore = "Manual real Telegram upload comparison; requires TGDRIVE_COMPARE_FILE and an authenticated local session"]
async fn compare_real_upload_chunk_profiles() {
    let source_path = std::env::var("TGDRIVE_COMPARE_FILE")
        .map(PathBuf::from)
        .expect("set TGDRIVE_COMPARE_FILE to the absolute file path for comparison");
    let source_meta = std::fs::metadata(&source_path).expect("failed to stat TGDRIVE_COMPARE_FILE");
    let data_root = dirs::data_dir()
        .expect("data_dir unavailable")
        .join("telegram-drive");
    let real_db = Database::open(&data_root.join("telegram-drive.db"))
        .expect("failed to open real telegram-drive.db");
    let session_blob = real_db
        .load_session_blob("primary")
        .expect("failed to read stored session blob")
        .expect("no stored session blob found");
    let auth_prefill = real_db
        .get_setting_json::<crate::models::AuthPrefillDto>("auth.prefill")
        .expect("failed to read auth.prefill")
        .expect("no auth.prefill found");

    let temp = tempdir().unwrap();
    let temp_db = Database::open(&temp.path().join("compare.db")).unwrap();
    temp_db.save_session_blob("primary", &session_blob).unwrap();
    temp_db
        .set_setting_json("auth.prefill", &auth_prefill)
        .unwrap();

    let telegram = TelegramClient::new(data_root.join("telegram_saved_messages"))
        .await
        .expect("failed to build telegram client");
    let auth = AuthService::new(temp_db.clone(), telegram.clone());
    let restored = auth
        .restore_session()
        .await
        .expect("session restore failed");
    assert!(matches!(restored, crate::models::AuthState::LoggedIn));

    let mut key = [0u8; 32];
    let digest = sha2::Sha256::digest(
        format!("telegram-drive-compare:{}", source_path.display()).as_bytes(),
    );
    key.copy_from_slice(&digest[..32]);

    let progress = ProgressHub::new();
    let log_dir = default_log_dir();
    std::fs::create_dir_all(&log_dir).unwrap();
    let jsonl_path = log_dir.join("real-upload-compare.jsonl");
    let summary_path = log_dir.join("real-upload-compare.md");
    let _ = std::fs::remove_file(&jsonl_path);
    let _ = std::fs::remove_file(&summary_path);

    let chunk_sizes = [
        crate::models::CHUNK_SIZE_128_MIB,
        crate::models::CHUNK_SIZE_256_MIB,
    ];
    let mut records = Vec::new();

    for chunk_size in chunk_sizes {
        let db_for_run =
            Database::open(&temp.path().join(format!("compare-{chunk_size}.db"))).unwrap();
        db_for_run
            .save_session_blob("primary", &session_blob)
            .unwrap();
        db_for_run
            .set_setting_json("auth.prefill", &auth_prefill)
            .unwrap();
        let dedup = DedupEngine::new(db_for_run.clone());
        let cache = LocalCdnCache::new(
            temp.path().join(format!("compare-cache-{chunk_size}")),
            8 * 1024 * 1024 * 1024,
        )
        .await
        .unwrap();
        let uploader = UploadService::new(
            db_for_run.clone(),
            dedup,
            ChunkingEngine::new(chunk_size, key),
            telegram.clone(),
            cache,
            progress.clone(),
        );
        let run_root_id = db_for_run.root_folder_id().unwrap();

        let upload_start = Instant::now();
        let file_id = uploader
            .upload_file(run_root_id, source_path.clone(), 8, chunk_size)
            .await
            .expect("real upload comparison failed");
        let upload_ms = upload_start.elapsed().as_millis();
        let file = db_for_run.get_file(file_id).unwrap();
        let chunk_count = db_for_run.get_chunks_for_file(file_id).unwrap().len();
        let mib_per_sec =
            source_meta.len() as f64 / 1024_f64 / 1024_f64 / (upload_ms.max(1) as f64 / 1000.0);

        let record = RealUploadCompareRecord {
            chunk_size_bytes: chunk_size,
            bytes: source_meta.len(),
            upload_ms,
            mib_per_sec,
            chunk_count,
            storage_mode: format!("{:?}", file.storage_mode),
            file_name: source_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown")
                .to_string(),
        };
        append_json_line(&jsonl_path, &record).unwrap();
        records.push(record);
    }

    write_real_upload_compare_summary(&summary_path, &records).unwrap();
}

fn benchmark_fixture_bytes() -> u64 {
    const DEFAULT_BYTES: u64 = 256 * 1024 * 1024;
    std::env::var("TGDRIVE_BENCH_BYTES")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|bytes| *bytes >= 64 * 1024 * 1024)
        .unwrap_or(DEFAULT_BYTES)
}

async fn benchmark_chunk_preparation(chunking: &ChunkingEngine, path: &Path) -> (u64, usize) {
    let mut file = File::open(path).await.unwrap();
    let mut buffer = vec![0u8; chunking.chunk_size()];
    let mut total_bytes = 0u64;
    let mut filled = 0usize;
    let mut chunk_count = 0usize;

    loop {
        let n = file.read(&mut buffer[filled..]).await.unwrap();
        if n == 0 {
            if filled == 0 {
                break;
            }
        } else {
            filled += n;
            total_bytes += n as u64;
        }

        if filled < chunking.chunk_size() && n != 0 {
            continue;
        }

        let _ = chunking.encrypt_chunk(&buffer[..filled]).unwrap();
        chunk_count += 1;
        filled = 0;
    }

    (total_bytes, chunk_count)
}

fn default_log_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("logs")
        .join("perf")
}

fn append_json_line<T: Serialize>(path: &Path, record: &T) -> std::io::Result<()> {
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

fn write_chunk_benchmark_summary(
    path: &Path,
    records: &[ChunkBenchmarkRecord],
) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)?;
    writeln!(file, "# Chunk Benchmark Summary")?;
    writeln!(file)?;
    for record in records {
        writeln!(
            file,
            "- chunk={} MiB | bytes={} | prep={}ms | throughput={:.2} MiB/s | chunks={}",
            record.chunk_size_bytes / 1024 / 1024,
            record.bytes,
            record.prep_ms,
            record.mib_per_sec,
            record.chunk_count
        )?;
    }
    Ok(())
}

fn write_real_upload_compare_summary(
    path: &Path,
    records: &[RealUploadCompareRecord],
) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)?;
    writeln!(file, "# Real Upload Compare")?;
    writeln!(file)?;
    for record in records {
        writeln!(
            file,
            "- file={} | chunk={} MiB | bytes={} | upload={}ms | throughput={:.2} MiB/s | chunks={} | mode={}",
            record.file_name,
            record.chunk_size_bytes / 1024 / 1024,
            record.bytes,
            record.upload_ms,
            record.mib_per_sec,
            record.chunk_count,
            record.storage_mode
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
