#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod auth;
mod cache;
mod chunking;
mod database;
mod dedup;
mod downloader;
mod file_index;
mod models;
mod progress;
mod session_store;
mod telegram;
mod uploader;

use auth::AuthService;
use cache::LocalCdnCache;
use chunking::ChunkingEngine;
use database::Database;
use dedup::DedupEngine;
use downloader::DownloadService;
use file_index::FileIndexService;
use models::{
    ApiResponse, AuthStartInput, AuthState, EntryKind, MoveInput, RenameInput, SearchQuery, SettingsDto,
};
use progress::ProgressHub;
use sha2::Digest;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{Emitter, State};
use telegram::TelegramClient;
use tracing::{error, info};
use uploader::UploadService;
use walkdir::WalkDir;

#[derive(Clone)]
struct AppState {
    db: Database,
    auth: AuthService,
    index: FileIndexService,
    uploader: UploadService,
    downloader: DownloadService,
    progress: ProgressHub,
}

#[tauri::command]
#[tracing::instrument(skip(state, input), name = "auth_start")]
async fn auth_start(state: State<'_, AppState>, input: AuthStartInput) -> Result<ApiResponse<AuthState>, String> {
    Ok(map_response(
        state
            .auth
            .start_phone_auth(input.phone, input.api_id, input.api_hash)
            .await,
    ))
}

#[tauri::command]
#[tracing::instrument(skip(state, code), name = "auth_verify_code")]
async fn auth_verify_code(state: State<'_, AppState>, code: String) -> Result<ApiResponse<AuthState>, String> {
    Ok(map_response(state.auth.verify_code(code).await))
}

#[tauri::command]
#[tracing::instrument(skip(state, password), name = "auth_verify_password")]
async fn auth_verify_password(state: State<'_, AppState>, password: String) -> Result<ApiResponse<AuthState>, String> {
    Ok(map_response(state.auth.verify_password(password).await))
}

#[tauri::command]
#[tracing::instrument(skip(state), name = "auth_status")]
async fn auth_status(state: State<'_, AppState>) -> Result<ApiResponse<AuthState>, String> {
    Ok(ApiResponse::ok(state.auth.status()))
}

#[tauri::command]
async fn list_folder(
    state: State<'_, AppState>,
    folder_id: i64,
    page: u32,
    page_size: u32,
    _sort: Option<String>,
    _direction: Option<String>,
) -> Result<ApiResponse<models::FolderListResponse>, String> {
    Ok(map_response(state.index.list_folder(folder_id, page, page_size)))
}

#[tauri::command]
async fn create_folder(state: State<'_, AppState>, parent_id: Option<i64>, name: String) -> Result<ApiResponse<models::Folder>, String> {
    Ok(map_response(state.index.create_folder(parent_id, name)))
}

#[tauri::command]
async fn rename_entry(state: State<'_, AppState>, input: RenameInput) -> Result<ApiResponse<()>, String> {
    let is_folder = matches!(input.kind, EntryKind::Folder);
    Ok(map_response(state.db.rename_entry(input.entry_id, &input.new_name, is_folder)))
}

#[tauri::command]
async fn move_entry(state: State<'_, AppState>, input: MoveInput) -> Result<ApiResponse<()>, String> {
    let is_folder = matches!(input.kind, EntryKind::Folder);
    Ok(map_response(state.db.move_entry(input.entry_id, input.target_folder_id, is_folder)))
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

#[tauri::command]
async fn upload_files(state: State<'_, AppState>, folder_id: i64, paths: Vec<String>) -> Result<ApiResponse<Vec<i64>>, String> {
    let path_bufs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    Ok(upload_path_list(state.inner(), folder_id, path_bufs).await)
}

#[tauri::command]
async fn upload_folder(
    state: State<'_, AppState>,
    folder_id: i64,
    directory_path: String,
) -> Result<ApiResponse<Vec<i64>>, String> {
    let root = PathBuf::from(&directory_path);
    if !root.exists() || !root.is_dir() {
        return Ok(ApiResponse::err("invalid directory path".to_string()));
    }

    let mut files = Vec::new();
    for entry in WalkDir::new(&root).into_iter().flatten() {
        let path = entry.path();
        if path.is_file() {
            files.push(path.to_path_buf());
        }
    }
    files.sort();
    if files.is_empty() {
        return Ok(ApiResponse::err("selected folder has no files".to_string()));
    }

    Ok(upload_path_list(state.inner(), folder_id, files).await)
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

async fn upload_path_list(state: &AppState, folder_id: i64, paths: Vec<PathBuf>) -> ApiResponse<Vec<i64>> {
    let settings = state.db.load_settings().unwrap_or_default();
    let mut ids = Vec::new();
    for p in paths {
        let result = state
            .uploader
            .upload_file(folder_id, p.clone(), settings.max_parallelism)
            .await;
        match result {
            Ok(id) => ids.push(id),
            Err(e) => {
                error!(path = %p.display(), error = %e, "upload failed");
                return ApiResponse::err(e.to_string());
            }
        }
    }
    ApiResponse::ok(ids)
}

#[tauri::command]
async fn download_file(
    state: State<'_, AppState>,
    file_id: i64,
    destination_path: String,
) -> Result<ApiResponse<()>, String> {
    let settings = state.db.load_settings().unwrap_or_default();
    Ok(map_response(
        state
            .downloader
            .download_file(file_id, PathBuf::from(destination_path), settings.max_parallelism)
            .await,
    ))
}

#[tauri::command]
async fn preview_image(state: State<'_, AppState>, file_id: i64) -> Result<ApiResponse<models::PreviewResponse>, String> {
    Ok(map_response(state.downloader.materialize_preview(file_id).await))
}

#[tauri::command]
async fn transfer_cancel(state: State<'_, AppState>, job_id: String) -> Result<ApiResponse<()>, String> {
    state.progress.cancel(&job_id);
    Ok(ApiResponse::ok(()))
}

#[tauri::command]
async fn settings_get(state: State<'_, AppState>) -> Result<ApiResponse<SettingsDto>, String> {
    match state.db.load_settings() {
        Ok(s) => Ok(ApiResponse::ok(s)),
        Err(e) => Ok(ApiResponse::err(e.to_string())),
    }
}

#[tauri::command]
async fn settings_set(state: State<'_, AppState>, settings: SettingsDto) -> Result<ApiResponse<()>, String> {
    Ok(map_response(state.db.set_setting_json("app.settings", &settings)))
}

#[tauri::command]
async fn folder_tree(state: State<'_, AppState>) -> Result<ApiResponse<Vec<models::Folder>>, String> {
    Ok(map_response(state.index.list_tree()))
}

fn map_response<T>(res: Result<T, models::AppError>) -> ApiResponse<T> {
    match res {
        Ok(v) => ApiResponse::ok(v),
        Err(e) => ApiResponse::err(e.to_string()),
    }
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "telegram_drive=info,info".into()),
        )
        .compact()
        .init();
}

fn app_data_root() -> models::AppResult<PathBuf> {
    let mut path = dirs::data_dir()
        .ok_or_else(|| models::AppError::Validation("unable to resolve data directory".to_string()))?;
    path.push("telegram-drive");
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

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

    let app = tauri::Builder::default()
        .manage(shared_state)
        .invoke_handler(tauri::generate_handler![
            auth_start,
            auth_verify_code,
            auth_verify_password,
            auth_status,
            list_folder,
            create_folder,
            rename_entry,
            move_entry,
            search,
            upload_files,
            upload_folder,
            pick_files_native,
            pick_folder_native,
            download_file,
            preview_image,
            transfer_cancel,
            settings_get,
            settings_set,
            folder_tree
        ])
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let mut rx = state_for_events.progress.subscribe();
            tauri::async_runtime::spawn(async move {
                while let Ok(status) = rx.recv().await {
                    let _ = app_handle.emit("transfer_progress", &status);
                    let _ = app_handle.emit("transfer_state_changed", &status);
                }
            });
            let _ = app.handle().emit("auth_state_changed", state_for_events.auth.status());
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
