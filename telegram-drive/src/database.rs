use crate::models::{
    AppError, AppResult, FileEntry, Folder, FolderListResponse, SearchQuery, SettingsDto,
    StorageMode,
};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct ChunkIndexRow {
    pub hash: String,
    pub telegram_file_id: String,
    pub size: i64,
    pub ref_count: i64,
    pub nonce_b64: String,
}

#[derive(Debug, Clone)]
pub struct NewFileRecord {
    pub name: String,
    pub size: i64,
    pub hash: String,
    pub folder_id: i64,
    pub mime_type: String,
    pub original_path: Option<String>,
    pub storage_mode: StorageMode,
    pub telegram_file_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewChunkRecord {
    pub part_index: i64,
    pub hash: String,
    pub telegram_file_id: String,
    pub size: i64,
    pub nonce_b64: String,
}

#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    pub fn open(path: &Path) -> AppResult<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            PRAGMA foreign_keys=ON;
            PRAGMA busy_timeout=8000;
            PRAGMA synchronous=NORMAL;
            PRAGMA temp_store=MEMORY;
            ",
        )?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.migrate()?;
        db.ensure_root_folder()?;
        Ok(db)
    }

    fn lock_conn(&self) -> AppResult<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| AppError::Concurrency("database mutex poisoned".to_string()))
    }

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    fn migrate(&self) -> AppResult<()> {
        let conn = self.lock_conn()?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS folders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                parent_id INTEGER NULL REFERENCES folders(id) ON DELETE CASCADE,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                size INTEGER NOT NULL,
                hash TEXT NOT NULL,
                folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
                mime_type TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                original_path TEXT NULL,
                storage_mode TEXT NOT NULL DEFAULT 'chunked',
                telegram_file_id TEXT NULL
            );

            CREATE TABLE IF NOT EXISTS chunks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
                part_index INTEGER NOT NULL,
                hash TEXT NOT NULL,
                telegram_file_id TEXT NOT NULL,
                size INTEGER NOT NULL,
                nonce_b64 TEXT NOT NULL,
                created_at TEXT NOT NULL,
                UNIQUE(file_id, part_index)
            );

            CREATE TABLE IF NOT EXISTS chunk_index (
                hash TEXT PRIMARY KEY,
                telegram_file_id TEXT NOT NULL,
                size INTEGER NOT NULL,
                ref_count INTEGER NOT NULL,
                nonce_b64 TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS file_hash_index (
                hash TEXT PRIMARY KEY,
                canonical_file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
                ref_count INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS file_refs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
                target_file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                blob BLOB NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_folders_parent ON folders(parent_id);
            CREATE INDEX IF NOT EXISTS idx_files_folder ON files(folder_id);
            CREATE INDEX IF NOT EXISTS idx_files_name ON files(name);
            CREATE INDEX IF NOT EXISTS idx_files_hash ON files(hash);
            CREATE INDEX IF NOT EXISTS idx_chunks_file ON chunks(file_id);
            CREATE INDEX IF NOT EXISTS idx_chunks_hash ON chunks(hash);
            CREATE INDEX IF NOT EXISTS idx_chunks_part ON chunks(file_id, part_index);
            ",
        )?;

        if !has_column(&conn, "files", "storage_mode")? {
            conn.execute(
                "ALTER TABLE files ADD COLUMN storage_mode TEXT NOT NULL DEFAULT 'chunked'",
                [],
            )?;
        }
        if !has_column(&conn, "files", "telegram_file_id")? {
            conn.execute(
                "ALTER TABLE files ADD COLUMN telegram_file_id TEXT NULL",
                [],
            )?;
        }
        if !has_column(&conn, "chunk_index", "nonce_b64")? {
            conn.execute(
                "ALTER TABLE chunk_index ADD COLUMN nonce_b64 TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_files_storage_mode ON files(storage_mode)",
            [],
        )?;
        Ok(())
    }

    fn ensure_root_folder(&self) -> AppResult<()> {
        let conn = self.lock_conn()?;
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM folders WHERE parent_id IS NULL LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_none() {
            let now = Self::now().to_rfc3339();
            conn.execute(
                "INSERT INTO folders(name, parent_id, created_at, updated_at) VALUES(?1, NULL, ?2, ?3)",
                params!["Root", now, now],
            )?;
        }
        Ok(())
    }

    pub fn root_folder_id(&self) -> AppResult<i64> {
        let conn = self.lock_conn()?;
        let id: i64 = conn.query_row(
            "SELECT id FROM folders WHERE parent_id IS NULL ORDER BY id ASC LIMIT 1",
            [],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    pub fn create_folder(&self, parent_id: Option<i64>, name: &str) -> AppResult<Folder> {
        let now = Self::now().to_rfc3339();
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO folders(name, parent_id, created_at, updated_at) VALUES(?1, ?2, ?3, ?4)",
            params![name, parent_id, now, now],
        )?;
        let id = conn.last_insert_rowid();
        drop(conn);
        self.get_folder(id)
    }

    pub fn get_folder(&self, id: i64) -> AppResult<Folder> {
        let conn = self.lock_conn()?;
        conn.query_row(
            "SELECT id, name, parent_id, created_at, updated_at FROM folders WHERE id=?1",
            params![id],
            Self::folder_from_row,
        )
        .map_err(AppError::from)
    }

    fn folder_from_row(row: &Row<'_>) -> rusqlite::Result<Folder> {
        Ok(Folder {
            id: row.get(0)?,
            name: row.get(1)?,
            parent_id: row.get(2)?,
            created_at: parse_ts(row.get::<_, String>(3)?)?,
            updated_at: parse_ts(row.get::<_, String>(4)?)?,
        })
    }

    fn file_from_row(row: &Row<'_>) -> rusqlite::Result<FileEntry> {
        Ok(FileEntry {
            id: row.get(0)?,
            name: row.get(1)?,
            size: row.get(2)?,
            hash: row.get(3)?,
            folder_id: row.get(4)?,
            mime_type: row.get(5)?,
            created_at: parse_ts(row.get::<_, String>(6)?)?,
            updated_at: parse_ts(row.get::<_, String>(7)?)?,
            original_path: row.get(8)?,
            storage_mode: parse_storage_mode(row.get::<_, String>(9)?)?,
            telegram_file_id: row.get(10)?,
        })
    }

    pub fn list_folder(
        &self,
        folder_id: i64,
        page: u32,
        page_size: u32,
    ) -> AppResult<FolderListResponse> {
        let offset = (page * page_size) as i64;
        let limit = page_size as i64;
        let conn = self.lock_conn()?;

        let folders = {
            let mut stmt = conn.prepare(
                "SELECT id, name, parent_id, created_at, updated_at
                 FROM folders WHERE parent_id = ?1 ORDER BY name ASC LIMIT ?2 OFFSET ?3",
            )?;
            let rows = stmt.query_map(params![folder_id, limit, offset], Self::folder_from_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        let files = {
            let mut stmt = conn.prepare(
                "SELECT id, name, size, hash, folder_id, mime_type, created_at, updated_at, original_path, storage_mode, telegram_file_id
                 FROM files WHERE folder_id = ?1 ORDER BY name ASC LIMIT ?2 OFFSET ?3",
            )?;
            let rows = stmt.query_map(params![folder_id, limit, offset], Self::file_from_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        let total_folders: u64 = conn.query_row(
            "SELECT COUNT(1) FROM folders WHERE parent_id = ?1",
            params![folder_id],
            |row| row.get(0),
        )?;
        let total_files: u64 = conn.query_row(
            "SELECT COUNT(1) FROM files WHERE folder_id = ?1",
            params![folder_id],
            |row| row.get(0),
        )?;

        Ok(FolderListResponse {
            folders,
            files,
            total_folders,
            total_files,
        })
    }

    pub fn search(&self, query: SearchQuery) -> AppResult<FolderListResponse> {
        let offset = (query.page * query.page_size) as i64;
        let limit = query.page_size as i64;
        let needle = format!("%{}%", query.query);
        let conn = self.lock_conn()?;

        let files_sql_folder = "SELECT id, name, size, hash, folder_id, mime_type, created_at, updated_at, original_path, storage_mode, telegram_file_id
             FROM files WHERE folder_id = ?1 AND name LIKE ?2 ORDER BY name ASC LIMIT ?3 OFFSET ?4";
        let files_sql_all = "SELECT id, name, size, hash, folder_id, mime_type, created_at, updated_at, original_path, storage_mode, telegram_file_id
             FROM files WHERE name LIKE ?1 ORDER BY name ASC LIMIT ?2 OFFSET ?3";

        let files = if let Some(fid) = query.folder_id {
            let mut stmt = conn.prepare(files_sql_folder)?;
            let rows = stmt.query_map(params![fid, needle, limit, offset], Self::file_from_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
        } else {
            let mut stmt = conn.prepare(files_sql_all)?;
            let rows = stmt.query_map(params![needle, limit, offset], Self::file_from_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        Ok(FolderListResponse {
            folders: Vec::new(),
            files,
            total_folders: 0,
            total_files: 0,
        })
    }

    pub fn resolve_conflict_name(&self, folder_id: i64, original: &str) -> AppResult<String> {
        let conn = self.lock_conn()?;
        let mut name = original.to_string();
        let mut n = 1u32;
        while conn
            .query_row(
                "SELECT 1 FROM files WHERE folder_id = ?1 AND name = ?2 LIMIT 1",
                params![folder_id, name],
                |row| row.get::<_, i32>(0),
            )
            .optional()?
            .is_some()
        {
            name = append_suffix(original, n);
            n += 1;
        }
        Ok(name)
    }

    pub fn find_file_by_hash(&self, hash: &str) -> AppResult<Option<FileEntry>> {
        let conn = self.lock_conn()?;
        let canonical_id: Option<i64> = conn
            .query_row(
                "SELECT canonical_file_id FROM file_hash_index WHERE hash=?1",
                params![hash],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = canonical_id {
            drop(conn);
            return self.get_file(id).map(Some);
        }
        Ok(None)
    }

    pub fn get_file(&self, id: i64) -> AppResult<FileEntry> {
        let conn = self.lock_conn()?;
        conn.query_row(
            "SELECT id, name, size, hash, folder_id, mime_type, created_at, updated_at, original_path, storage_mode, telegram_file_id FROM files WHERE id=?1",
            params![id],
            Self::file_from_row,
        )
        .map_err(AppError::from)
    }

    pub fn get_chunks_for_file(
        &self,
        file_id: i64,
    ) -> AppResult<Vec<(i64, String, String, i64, String)>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT part_index, hash, telegram_file_id, size, nonce_b64
             FROM chunks WHERE file_id = ?1 ORDER BY part_index ASC",
        )?;
        let rows = stmt.query_map(params![file_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn get_chunk_index(&self, hash: &str) -> AppResult<Option<ChunkIndexRow>> {
        let conn = self.lock_conn()?;
        let row = conn
            .query_row(
                "SELECT hash, telegram_file_id, size, ref_count, nonce_b64 FROM chunk_index WHERE hash = ?1",
                params![hash],
                |r| {
                    Ok(ChunkIndexRow {
                        hash: r.get(0)?,
                        telegram_file_id: r.get(1)?,
                        size: r.get(2)?,
                        ref_count: r.get(3)?,
                        nonce_b64: r.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    pub fn persist_uploaded_file(
        &self,
        file: NewFileRecord,
        chunks: Vec<NewChunkRecord>,
    ) -> AppResult<i64> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction()?;
        let now = Self::now().to_rfc3339();
        tx.execute(
            "INSERT INTO files(name, size, hash, folder_id, mime_type, created_at, updated_at, original_path, storage_mode, telegram_file_id)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                file.name,
                file.size,
                file.hash,
                file.folder_id,
                file.mime_type,
                now,
                now,
                file.original_path,
                storage_mode_to_str(&file.storage_mode),
                file.telegram_file_id
            ],
        )?;
        let file_id = tx.last_insert_rowid();

        for chunk in &chunks {
            tx.execute(
                "INSERT INTO chunks(file_id, part_index, hash, telegram_file_id, size, nonce_b64, created_at)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    file_id,
                    chunk.part_index,
                    chunk.hash,
                    chunk.telegram_file_id,
                    chunk.size,
                    chunk.nonce_b64,
                    now
                ],
            )?;
            tx.execute(
                "INSERT INTO chunk_index(hash, telegram_file_id, size, ref_count, nonce_b64, created_at, updated_at)
                 VALUES(?1, ?2, ?3, 1, ?4, ?5, ?6)
                 ON CONFLICT(hash) DO UPDATE SET ref_count = chunk_index.ref_count + 1, updated_at = excluded.updated_at",
                params![chunk.hash, chunk.telegram_file_id, chunk.size, chunk.nonce_b64, now, now],
            )?;
        }

        tx.execute(
            "INSERT INTO file_hash_index(hash, canonical_file_id, ref_count, created_at, updated_at)
             VALUES(?1, ?2, 1, ?3, ?4)
             ON CONFLICT(hash) DO UPDATE SET ref_count = file_hash_index.ref_count + 1, updated_at = excluded.updated_at",
            params![file.hash, file_id, now, now],
        )?;

        tx.commit()?;
        Ok(file_id)
    }

    pub fn create_file_reference(
        &self,
        source_file_id: i64,
        destination_folder_id: i64,
        destination_name: &str,
    ) -> AppResult<i64> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction()?;
        let source: (i64, String, i64, String, String, Option<String>, String, Option<String>) =
            tx.query_row(
                "SELECT id, hash, size, mime_type, created_at, original_path, storage_mode, telegram_file_id FROM files WHERE id=?1",
                params![source_file_id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                    ))
                },
            )?;
        let now = Self::now().to_rfc3339();
        tx.execute(
            "INSERT INTO files(name, size, hash, folder_id, mime_type, created_at, updated_at, original_path, storage_mode, telegram_file_id)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                destination_name,
                source.2,
                source.1,
                destination_folder_id,
                source.3,
                source.4,
                now,
                source.5,
                source.6,
                source.7
            ],
        )?;
        let new_file_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO file_refs(file_id, target_file_id, created_at) VALUES(?1, ?2, ?3)",
            params![new_file_id, source_file_id, now],
        )?;

        {
            let mut stmt = tx.prepare(
                "SELECT part_index, hash, telegram_file_id, size, nonce_b64 FROM chunks WHERE file_id = ?1 ORDER BY part_index ASC",
            )?;
            let chunk_rows = stmt
                .query_map(params![source_file_id], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            for (part_index, hash, telegram_file_id, size, nonce_b64) in chunk_rows {
                tx.execute(
                    "INSERT INTO chunks(file_id, part_index, hash, telegram_file_id, size, nonce_b64, created_at)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![new_file_id, part_index, hash, telegram_file_id, size, nonce_b64, now],
                )?;
                tx.execute(
                    "UPDATE chunk_index SET ref_count = ref_count + 1, updated_at = ?2 WHERE hash = ?1",
                    params![hash, now],
                )?;
            }
        }

        tx.execute(
            "UPDATE file_hash_index SET ref_count = ref_count + 1, updated_at = ?2 WHERE hash = ?1",
            params![source.1, now],
        )?;
        tx.commit()?;
        Ok(new_file_id)
    }

    pub fn rename_entry(&self, id: i64, new_name: &str, is_folder: bool) -> AppResult<()> {
        let now = Self::now().to_rfc3339();
        let conn = self.lock_conn()?;
        if is_folder {
            conn.execute(
                "UPDATE folders SET name=?1, updated_at=?2 WHERE id=?3",
                params![new_name, now, id],
            )?;
        } else {
            conn.execute(
                "UPDATE files SET name=?1, updated_at=?2 WHERE id=?3",
                params![new_name, now, id],
            )?;
        }
        Ok(())
    }

    pub fn move_entry(&self, id: i64, target_folder_id: i64, is_folder: bool) -> AppResult<()> {
        let now = Self::now().to_rfc3339();
        let conn = self.lock_conn()?;
        if is_folder {
            conn.execute(
                "UPDATE folders SET parent_id=?1, updated_at=?2 WHERE id=?3",
                params![target_folder_id, now, id],
            )?;
        } else {
            conn.execute(
                "UPDATE files SET folder_id=?1, updated_at=?2 WHERE id=?3",
                params![target_folder_id, now, id],
            )?;
        }
        Ok(())
    }

    pub fn delete_file(&self, file_id: i64) -> AppResult<()> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction()?;
        let now = Self::now().to_rfc3339();

        let file_hash: String = tx.query_row(
            "SELECT hash FROM files WHERE id = ?1",
            params![file_id],
            |r| r.get(0),
        )?;

        {
            let mut stmt = tx.prepare("SELECT hash FROM chunks WHERE file_id = ?1")?;
            let hashes: Vec<String> = stmt
                .query_map(params![file_id], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            for hash in hashes {
                tx.execute(
                    "UPDATE chunk_index SET ref_count = ref_count - 1, updated_at = ?2 WHERE hash = ?1",
                    params![hash, now],
                )?;
                tx.execute(
                    "DELETE FROM chunk_index WHERE hash = ?1 AND ref_count <= 0",
                    params![hash],
                )?;
            }
        }

        tx.execute(
            "UPDATE file_hash_index SET ref_count = ref_count - 1, updated_at = ?2 WHERE hash = ?1",
            params![file_hash, now],
        )?;
        tx.execute(
            "DELETE FROM file_hash_index WHERE hash = ?1 AND ref_count <= 0",
            params![file_hash],
        )?;

        tx.execute("DELETE FROM files WHERE id = ?1", params![file_id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn delete_folder(&self, folder_id: i64) -> AppResult<()> {
        let is_root = {
            let conn = self.lock_conn()?;
            conn.query_row(
                "SELECT id FROM folders WHERE id = ?1 AND parent_id IS NULL",
                params![folder_id],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .is_some()
        };
        if is_root {
            return Err(AppError::Validation(
                "não é possível apagar a pasta raiz".to_string(),
            ));
        }

        let folder_ids = self.collect_descendant_folder_ids(folder_id)?;
        let file_ids = self.collect_files_in_folders(&folder_ids)?;

        for file_id in file_ids {
            self.delete_file(file_id)?;
        }

        let conn = self.lock_conn()?;
        conn.execute("DELETE FROM folders WHERE id = ?1", params![folder_id])?;
        Ok(())
    }

    fn collect_descendant_folder_ids(&self, root_folder_id: i64) -> AppResult<Vec<i64>> {
        let conn = self.lock_conn()?;
        let mut queue = VecDeque::from([root_folder_id]);
        let mut all = Vec::new();
        while let Some(folder_id) = queue.pop_front() {
            all.push(folder_id);
            let mut stmt = conn.prepare("SELECT id FROM folders WHERE parent_id = ?1")?;
            let children = stmt
                .query_map(params![folder_id], |r| r.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            for child in children {
                queue.push_back(child);
            }
        }
        Ok(all)
    }

    fn collect_files_in_folders(&self, folder_ids: &[i64]) -> AppResult<Vec<i64>> {
        let conn = self.lock_conn()?;
        let mut file_ids = Vec::new();
        for folder_id in folder_ids {
            let mut stmt = conn.prepare("SELECT id FROM files WHERE folder_id = ?1")?;
            let ids = stmt
                .query_map(params![folder_id], |r| r.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            file_ids.extend(ids);
        }
        Ok(file_ids)
    }

    pub fn set_setting_json<T: serde::Serialize>(&self, key: &str, value: &T) -> AppResult<()> {
        let conn = self.lock_conn()?;
        let json = serde_json::to_string(value)?;
        conn.execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, json],
        )?;
        Ok(())
    }

    pub fn get_setting_json<T: serde::de::DeserializeOwned>(
        &self,
        key: &str,
    ) -> AppResult<Option<T>> {
        let conn = self.lock_conn()?;
        let val: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        match val {
            Some(v) => Ok(Some(serde_json::from_str::<T>(&v)?)),
            None => Ok(None),
        }
    }

    pub fn load_settings(&self) -> AppResult<SettingsDto> {
        Ok(self
            .get_setting_json::<SettingsDto>("app.settings")?
            .unwrap_or_default()
            .normalized())
    }

    pub fn save_session_blob(&self, id: &str, blob: &[u8]) -> AppResult<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO sessions(id, blob, updated_at) VALUES(?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET blob = excluded.blob, updated_at = excluded.updated_at",
            params![id, blob, Self::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn load_session_blob(&self, id: &str) -> AppResult<Option<Vec<u8>>> {
        let conn = self.lock_conn()?;
        let row = conn
            .query_row(
                "SELECT blob FROM sessions WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(row)
    }

    pub fn delete_session_blob(&self, id: &str) -> AppResult<()> {
        let conn = self.lock_conn()?;
        conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn list_all_folders(&self) -> AppResult<Vec<Folder>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, parent_id, created_at, updated_at FROM folders ORDER BY parent_id ASC, name ASC",
        )?;
        let rows = stmt.query_map([], Self::folder_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

fn append_suffix(name: &str, n: u32) -> String {
    if let Some((base, ext)) = name.rsplit_once('.') {
        format!("{base} ({n}).{ext}")
    } else {
        format!("{name} ({n})")
    }
}

fn parse_ts(value: String) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                value.len(),
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })
}

fn has_column(conn: &Connection, table: &str, column: &str) -> AppResult<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for col in rows {
        if col? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn parse_storage_mode(value: String) -> rusqlite::Result<StorageMode> {
    match value.as_str() {
        "single" => Ok(StorageMode::Single),
        "chunked" => Ok(StorageMode::Chunked),
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            other.len(),
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown storage mode {other}"),
            )),
        )),
    }
}

fn storage_mode_to_str(mode: &StorageMode) -> &'static str {
    match mode {
        StorageMode::Single => "single",
        StorageMode::Chunked => "chunked",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_suffix() {
        assert_eq!(append_suffix("file.txt", 1), "file (1).txt");
        assert_eq!(append_suffix("archive", 2), "archive (2)");
    }
}
