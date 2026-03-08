#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod auth;
mod cache;
mod chunking;
mod database;
mod dedup;
mod downloader;
mod file_index;
mod models;
mod performance;
mod progress;
mod security;
mod session_store;
mod telegram;
#[cfg(test)]
mod transfer_matrix;
mod uploader;

use auth::AuthService;
use cache::LocalCdnCache;
use chunking::ChunkingEngine;
use database::Database;
use dedup::DedupEngine;
use downloader::DownloadService;
use file_index::FileIndexService;
use models::{
    ApiResponse, AuthPrefillDto, AuthStartInput, AuthState, DownloadCacheMode, DownloadResponse,
    EntryKind, MoveInput, RenameInput, SearchQuery, SettingsDto,
};
use performance::AppPerformanceController;
use progress::ProgressHub;
use sha2::Digest;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use tauri::{Emitter, State};
use telegram::TelegramClient;
use tracing::{error, info};
use uploader::UploadService;
use walkdir::WalkDir;

static LOG_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

#[derive(Clone)]
struct AppState {
    db: Database,
    auth: AuthService,
    index: FileIndexService,
    uploader: UploadService,
    downloader: DownloadService,
    progress: ProgressHub,
}

// ─── Auth ──────────────────────────────────────────────────────────────────

#[tauri::command]
async fn auth_start(
    state: State<'_, AppState>,
    input: AuthStartInput,
) -> Result<ApiResponse<AuthState>, String> {
    Ok(map_response(state.auth.start_phone_auth(input.phone).await))
}

#[tauri::command]
async fn auth_verify_code(
    state: State<'_, AppState>,
    code: String,
) -> Result<ApiResponse<AuthState>, String> {
    Ok(map_response(state.auth.verify_code(code).await))
}

#[tauri::command]
async fn auth_verify_password(
    state: State<'_, AppState>,
    password: String,
) -> Result<ApiResponse<AuthState>, String> {
    Ok(map_response(state.auth.verify_password(password).await))
}

#[tauri::command]
async fn auth_status(state: State<'_, AppState>) -> Result<ApiResponse<AuthState>, String> {
    Ok(ApiResponse::ok(state.auth.status()))
}

#[tauri::command]
async fn auth_profile(
    state: State<'_, AppState>,
) -> Result<ApiResponse<models::UserProfileDto>, String> {
    Ok(map_response(state.auth.profile().await))
}

#[tauri::command]
async fn auth_prefill(
    state: State<'_, AppState>,
) -> Result<ApiResponse<Option<AuthPrefillDto>>, String> {
    Ok(map_response(state.auth.prefill()))
}

#[tauri::command]
async fn auth_logout(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<ApiResponse<AuthState>, String> {
    let response = map_response(state.auth.logout().await);
    if response.ok {
        let _ = app.emit("auth_state_changed", AuthState::LoggedOut);
    }
    Ok(response)
}

#[tauri::command]
async fn sync_saved_messages_index(
    state: State<'_, AppState>,
) -> Result<ApiResponse<usize>, String> {
    Ok(map_response(state.auth.sync_saved_messages_index().await))
}

// ─── Listagem ──────────────────────────────────────────────────────────────

#[tauri::command]
async fn list_folder(
    state: State<'_, AppState>,
    folder_id: i64,
    page: u32,
    page_size: u32,
    _sort: Option<String>,
    _direction: Option<String>,
) -> Result<ApiResponse<models::FolderListResponse>, String> {
    Ok(map_response(
        state.index.list_folder(folder_id, page, page_size),
    ))
}

#[tauri::command]
async fn folder_tree(
    state: State<'_, AppState>,
) -> Result<ApiResponse<Vec<models::Folder>>, String> {
    Ok(map_response(state.index.list_tree()))
}

#[tauri::command]
async fn create_folder(
    state: State<'_, AppState>,
    parent_id: Option<i64>,
    name: String,
) -> Result<ApiResponse<models::Folder>, String> {
    Ok(map_response(state.index.create_folder(parent_id, name)))
}

#[tauri::command]
async fn rename_entry(
    state: State<'_, AppState>,
    input: RenameInput,
) -> Result<ApiResponse<()>, String> {
    let is_folder = matches!(input.kind, EntryKind::Folder);
    Ok(map_response(state.db.rename_entry(
        input.entry_id,
        &input.new_name,
        is_folder,
    )))
}

#[tauri::command]
async fn move_entry(
    state: State<'_, AppState>,
    input: MoveInput,
) -> Result<ApiResponse<()>, String> {
    let is_folder = matches!(input.kind, EntryKind::Folder);
    Ok(map_response(state.db.move_entry(
        input.entry_id,
        input.target_folder_id,
        is_folder,
    )))
}

#[tauri::command]
async fn search(
    state: State<'_, AppState>,
    query: String,
    folder_id_opt: Option<i64>,
    page: u32,
    page_size: u32,
) -> Result<ApiResponse<models::FolderListResponse>, String> {
    Ok(map_response(state.index.search(SearchQuery {
        query,
        folder_id: folder_id_opt,
        page,
        page_size,
    })))
}

// ─── Deleção ───────────────────────────────────────────────────────────────

/// Apaga um arquivo do banco de dados local.
/// Nota: os chunks no Telegram NÃO são deletados (limitação da API do Telegram).
/// O ref_count é decrementado corretamente para deduplicação futura.
#[tauri::command]
async fn delete_file(state: State<'_, AppState>, file_id: i64) -> Result<ApiResponse<()>, String> {
    Ok(map_response(state.db.delete_file(file_id)))
}

/// Apaga uma pasta e todo o seu conteúdo do banco local (cascata).
/// A pasta raiz não pode ser apagada.
#[tauri::command]
async fn delete_folder(
    state: State<'_, AppState>,
    folder_id: i64,
) -> Result<ApiResponse<()>, String> {
    Ok(map_response(state.db.delete_folder(folder_id)))
}

// ─── Upload ────────────────────────────────────────────────────────────────

#[tauri::command]
async fn upload_files(
    state: State<'_, AppState>,
    folder_id: i64,
    paths: Vec<String>,
) -> Result<ApiResponse<()>, String> {
    if paths.is_empty() {
        return Ok(ApiResponse::err("nenhum arquivo selecionado"));
    }
    let path_bufs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let inner = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        let settings = inner.db.load_settings().unwrap_or_default();
        for p in path_bufs {
            let result = inner
                .uploader
                .upload_file(
                    folder_id,
                    p.clone(),
                    settings.max_parallelism,
                    settings.chunk_size_bytes,
                )
                .await;
            if let Err(e) = result {
                error!(path = %p.display(), error = %e, "upload failed");
            }
        }
    });
    Ok(ApiResponse::ok(()))
}

#[tauri::command]
async fn upload_folder(
    state: State<'_, AppState>,
    folder_id: i64,
    directory_path: String,
) -> Result<ApiResponse<()>, String> {
    let root = PathBuf::from(&directory_path);
    if !root.exists() || !root.is_dir() {
        return Ok(ApiResponse::err("caminho de diretório inválido"));
    }
    let mut files: Vec<PathBuf> = WalkDir::new(&root)
        .into_iter()
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.path().to_path_buf())
        .collect();
    files.sort();
    if files.is_empty() {
        return Ok(ApiResponse::err("a pasta selecionada não contém arquivos"));
    }
    let inner = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        let settings = inner.db.load_settings().unwrap_or_default();
        for p in files {
            let result = inner
                .uploader
                .upload_file(
                    folder_id,
                    p.clone(),
                    settings.max_parallelism,
                    settings.chunk_size_bytes,
                )
                .await;
            if let Err(e) = result {
                error!(path = %p.display(), error = %e, "upload failed");
            }
        }
    });
    Ok(ApiResponse::ok(()))
}

#[tauri::command]
async fn pick_files_native() -> Result<ApiResponse<Vec<String>>, String> {
    let selected = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("Select files to upload")
            .pick_files()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(ApiResponse::ok(selected))
}

#[tauri::command]
async fn pick_folder_native() -> Result<ApiResponse<Option<String>>, String> {
    let selected = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("Select folder to upload")
            .pick_folder()
            .map(|p| p.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(ApiResponse::ok(selected))
}

#[tauri::command]
async fn pick_save_file_native(
    suggested_name: Option<String>,
) -> Result<ApiResponse<Option<String>>, String> {
    let selected = tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new().set_title("Select download destination");
        if let Some(name) = suggested_name.as_deref() {
            dialog = dialog.set_file_name(name);
        }
        dialog.save_file().map(|p| p.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(ApiResponse::ok(selected))
}

// ─── Download ──────────────────────────────────────────────────────────────

#[tauri::command]
async fn download_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    file_id: i64,
    destination_path: String,
    cache_mode: Option<String>,
) -> Result<ApiResponse<DownloadResponse>, String> {
    let settings = state.db.load_settings().unwrap_or_default();
    let requested_cache_mode = DownloadCacheMode::from_option_str(cache_mode.as_deref());
    Ok(map_response(
        state
            .downloader
            .download_file(
                file_id,
                PathBuf::from(destination_path),
                settings.max_parallelism,
                settings,
                requested_cache_mode,
                Some(app),
            )
            .await,
    ))
}

#[tauri::command]
async fn preview_image(
    state: State<'_, AppState>,
    file_id: i64,
) -> Result<ApiResponse<models::PreviewResponse>, String> {
    Ok(map_response(
        state.downloader.materialize_preview(file_id).await,
    ))
}

// ─── Progresso e configuração ──────────────────────────────────────────────

#[tauri::command]
async fn transfer_cancel(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<ApiResponse<()>, String> {
    state.progress.cancel(&job_id);
    Ok(ApiResponse::ok(()))
}

#[tauri::command]
async fn transfer_pause(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<ApiResponse<()>, String> {
    state.progress.pause(&job_id);
    Ok(ApiResponse::ok(()))
}

#[tauri::command]
async fn transfer_resume(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<ApiResponse<()>, String> {
    state.progress.resume(&job_id);
    Ok(ApiResponse::ok(()))
}

/// Retorna todos os transfers ativos (para o frontend reconstruir o estado após reconexão).
#[tauri::command]
async fn transfers_snapshot(
    state: State<'_, AppState>,
) -> Result<ApiResponse<Vec<models::TransferStatus>>, String> {
    Ok(ApiResponse::ok(state.progress.snapshot()))
}

#[tauri::command]
async fn settings_get(state: State<'_, AppState>) -> Result<ApiResponse<SettingsDto>, String> {
    match state.db.load_settings() {
        Ok(s) => Ok(ApiResponse::ok(s)),
        Err(e) => Ok(ApiResponse::err(e.to_string())),
    }
}

#[tauri::command]
async fn settings_set(
    state: State<'_, AppState>,
    settings: SettingsDto,
) -> Result<ApiResponse<()>, String> {
    Ok(map_response(
        state
            .db
            .set_setting_json("app.settings", &settings.normalized()),
    ))
}

// ─── Helpers ───────────────────────────────────────────────────────────────

fn map_response<T>(res: Result<T, models::AppError>) -> ApiResponse<T> {
    match res {
        Ok(v) => ApiResponse::ok(v),
        Err(e) => ApiResponse::err(e.to_string()),
    }
}

fn init_tracing() {
    let log_root = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("logs")
        .join("telegram");
    let _ = std::fs::create_dir_all(&log_root);
    let file_appender = tracing_appender::rolling::daily(log_root, "runtime.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    let _ = LOG_GUARD.set(guard);

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "telegram_drive=debug,info".into()),
        )
        .with_writer(non_blocking)
        .with_ansi(false)
        .compact()
        .init();
}

fn app_data_root() -> models::AppResult<PathBuf> {
    let mut path = dirs::data_dir().ok_or_else(|| {
        models::AppError::Validation("unable to resolve data directory".to_string())
    })?;
    path.push("telegram-drive");
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

// ─── main ──────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    init_tracing();

    let data_root = match app_data_root() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("failed to initialize data directory: {e}");
            return;
        }
    };

    let db_path = data_root.join("telegram-drive.db");
    let db = match Database::open(&db_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("failed to initialize database: {e}");
            return;
        }
    };

    let settings = db.load_settings().unwrap_or_default();
    let progress = ProgressHub::new();
    let performance = AppPerformanceController::new();

    let cache = match LocalCdnCache::new(data_root.join("cache"), 6 * 1024 * 1024 * 1024).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("failed to initialize cache: {e}");
            return;
        }
    };

    let mut key = [0u8; 32];
    let digest = sha2::Sha256::digest(format!("telegram-drive:{}", db_path.display()).as_bytes());
    key.copy_from_slice(&digest[..32]);
    let chunking = ChunkingEngine::new(settings.chunk_size_bytes, key);

    let telegram = match TelegramClient::new(data_root.join("telegram_saved_messages")).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("failed to initialize telegram client: {e}");
            return;
        }
    };

    let auth = AuthService::new(db.clone(), telegram.clone());
    if let Err(e) = auth.restore_session().await {
        error!(error = %e, "unable to restore auth session");
    }

    let dedup = DedupEngine::new(db.clone());
    let index = FileIndexService::new(db.clone());
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
        progress.clone(),
    );

    let shared_state = AppState {
        db: db.clone(),
        auth: auth.clone(),
        index,
        uploader,
        downloader,
        progress: progress.clone(),
    };
    let state_for_events = Arc::new(shared_state.clone());
    let performance_for_events = performance.clone();

    let app = tauri::Builder::default()
        .manage(shared_state)
        .invoke_handler(tauri::generate_handler![
            auth_start,
            auth_verify_code,
            auth_verify_password,
            auth_status,
            auth_profile,
            auth_prefill,
            auth_logout,
            sync_saved_messages_index,
            list_folder,
            folder_tree,
            create_folder,
            rename_entry,
            move_entry,
            search,
            delete_file,
            delete_folder,
            upload_files,
            upload_folder,
            pick_files_native,
            pick_folder_native,
            pick_save_file_native,
            download_file,
            preview_image,
            transfer_cancel,
            transfer_pause,
            transfer_resume,
            transfers_snapshot,
            settings_get,
            settings_set,
        ])
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let mut rx = state_for_events.progress.subscribe();
            let progress_for_perf = state_for_events.progress.clone();

            // Emite eventos de progresso para o frontend em tempo real.
            // O canal tem capacidade 8192 para não perder eventos durante pico de chunks paralelos.
            tauri::async_runtime::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(status) => {
                            let _ = app_handle.emit("transfer_progress", &status);
                            let _ = app_handle.emit("transfer_state_changed", &status);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            error!("progress event loop lagged by {n} messages");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });

            tauri::async_runtime::spawn(async move {
                let mut last_active = None;
                loop {
                    let active = progress_for_perf.has_active_transfers();
                    if last_active != Some(active) {
                        performance_for_events.set_transfer_mode(active);
                        last_active = Some(active);
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            });

            let _ = app
                .handle()
                .emit("auth_state_changed", state_for_events.auth.status());
            Ok(())
        })
        .build(tauri::generate_context!());

    match app {
        Ok(app) => {
            info!("telegram-drive initialized");
            app.run(|_app, _event| {});
        }
        Err(e) => {
            eprintln!("tauri build failed: {e}");
        }
    }
}
