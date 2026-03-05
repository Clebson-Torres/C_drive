use crate::database::{ChunkIndexRow, Database};
use crate::models::{AppError, AppResult, FileEntry};

#[derive(Clone)]
pub struct DedupEngine {
    db: Database,
}

impl DedupEngine {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn find_duplicate_file(&self, file_hash: &str) -> AppResult<Option<FileEntry>> {
        self.db.find_file_by_hash(file_hash)
    }

    pub fn find_duplicate_chunk(&self, chunk_hash: &str, chunk_size: i64) -> AppResult<Option<ChunkIndexRow>> {
        let row = self.db.get_chunk_index(chunk_hash)?;
        match row {
            Some(existing) if existing.size == chunk_size => Ok(Some(existing)),
            Some(existing) => Err(AppError::Validation(format!(
                "chunk hash collision with different size, hash={} existing_size={} new_size={}",
                existing.hash, existing.size, chunk_size
            ))),
            None => Ok(None),
        }
    }
}
