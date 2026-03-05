use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("crypto error: {0}")]
    Crypto(String),
    #[error("telegram error: {0}")]
    Telegram(String),
    #[error("validation error: {0}")]
    Validation(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("concurrency error: {0}")]
    Concurrency(String),
}

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EntryKind {
    File,
    Folder,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransferState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransferPhase {
    Queued,
    Hashing,
    Uploading,
    Downloading,
    Reassembling,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthState {
    LoggedOut,
    AwaitingCode,
    AwaitingPassword,
    LoggedIn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileType {
    Image,
    Video,
    Document,
    Audio,
    Archive,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StorageMode {
    Single,
    Chunked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub id: i64,
    pub name: String,
    pub size: i64,
    pub hash: String,
    pub folder_id: i64,
    pub mime_type: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub original_path: Option<String>,
    pub storage_mode: StorageMode,
    pub telegram_file_id: Option<String>,
}

/// Status de uma transferência (upload ou download).
/// `speed_bps` é a velocidade instantânea em bytes/s (0 quando não disponível).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferStatus {
    pub job_id: String,
    pub file_name: String,
    pub state: TransferState,
    pub phase: TransferPhase,
    pub storage_mode: Option<StorageMode>,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub error: Option<String>,
    pub speed_bps: u64,
    pub eta_seconds: Option<u64>,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkMeta {
    pub id: i64,
    pub file_id: i64,
    pub part_index: i64,
    pub hash: String,
    pub telegram_file_id: String,
    pub size: i64,
    pub nonce_b64: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkDescriptor {
    pub part_index: i64,
    pub hash: String,
    pub size: usize,
    pub nonce_b64: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadJob {
    pub job_id: String,
    pub file_path: PathBuf,
    pub folder_id: i64,
    pub state: TransferState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadJob {
    pub job_id: String,
    pub file_id: i64,
    pub destination: PathBuf,
    pub state: TransferState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub query: String,
    pub folder_id: Option<i64>,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderListResponse {
    pub folders: Vec<Folder>,
    pub files: Vec<FileEntry>,
    pub total_folders: u64,
    pub total_files: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStartInput {
    pub phone: String,
    pub api_id: i32,
    pub api_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthPrefillDto {
    pub phone: String,
    pub api_id: i32,
    pub api_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfileDto {
    pub display_name: String,
    pub username: Option<String>,
    pub phone_masked: Option<String>,
    pub avatar_path_opt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameInput {
    pub entry_id: i64,
    pub new_name: String,
    pub kind: EntryKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveInput {
    pub entry_id: i64,
    pub target_folder_id: i64,
    pub kind: EntryKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsDto {
    #[serde(default = "default_chunk_size_bytes")]
    pub chunk_size_bytes: usize,
    #[serde(default = "default_max_parallelism")]
    pub max_parallelism: usize,
    #[serde(default = "default_encrypt_chunks")]
    pub encrypt_chunks: bool,
}

impl Default for SettingsDto {
    fn default() -> Self {
        Self {
            chunk_size_bytes: default_chunk_size_bytes(),
            max_parallelism: default_max_parallelism(),
            encrypt_chunks: default_encrypt_chunks(),
        }
    }
}

pub const CHUNK_SIZE_8_MIB: usize = 8 * 1024 * 1024;
pub const CHUNK_SIZE_32_MIB: usize = 32 * 1024 * 1024;
pub const CHUNK_SIZE_64_MIB: usize = 64 * 1024 * 1024;
pub const ALLOWED_CHUNK_SIZES: [usize; 3] =
    [CHUNK_SIZE_8_MIB, CHUNK_SIZE_32_MIB, CHUNK_SIZE_64_MIB];

pub fn default_chunk_size_bytes() -> usize {
    CHUNK_SIZE_32_MIB
}

pub fn default_max_parallelism() -> usize {
    16
}

pub fn default_encrypt_chunks() -> bool {
    true
}

pub fn normalize_chunk_size_bytes(value: usize) -> usize {
    if ALLOWED_CHUNK_SIZES.contains(&value) {
        value
    } else {
        default_chunk_size_bytes()
    }
}

impl SettingsDto {
    pub fn normalized(mut self) -> Self {
        self.chunk_size_bytes = normalize_chunk_size_bytes(self.chunk_size_bytes);
        self.max_parallelism = self.max_parallelism.clamp(1, 48);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewResponse {
    pub local_path: String,
    pub mime_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub ok: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(msg.into()),
        }
    }
}

pub fn classify_mime(path: &std::path::Path) -> String {
    mime_guess::from_path(path)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_string()
}

pub fn file_type_from_mime(mime: &str) -> FileType {
    if mime.starts_with("image/") {
        FileType::Image
    } else if mime.starts_with("video/") {
        FileType::Video
    } else if mime.starts_with("audio/") {
        FileType::Audio
    } else if mime.contains("zip") || mime.contains("compressed") {
        FileType::Archive
    } else if mime != "application/octet-stream" {
        FileType::Document
    } else {
        FileType::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_chunk_size_bytes, CHUNK_SIZE_32_MIB, CHUNK_SIZE_64_MIB, CHUNK_SIZE_8_MIB,
    };

    #[test]
    fn normalize_chunk_size_accepts_only_supported_options() {
        assert_eq!(
            normalize_chunk_size_bytes(CHUNK_SIZE_8_MIB),
            CHUNK_SIZE_8_MIB
        );
        assert_eq!(
            normalize_chunk_size_bytes(CHUNK_SIZE_32_MIB),
            CHUNK_SIZE_32_MIB
        );
        assert_eq!(
            normalize_chunk_size_bytes(CHUNK_SIZE_64_MIB),
            CHUNK_SIZE_64_MIB
        );
        assert_eq!(
            normalize_chunk_size_bytes(12 * 1024 * 1024),
            CHUNK_SIZE_32_MIB
        );
    }
}
