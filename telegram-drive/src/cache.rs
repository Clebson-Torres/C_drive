use crate::models::{AppError, AppResult};
use moka::future::Cache;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::fs;

#[derive(Clone)]
pub struct LocalCdnCache {
    cache_dir: PathBuf,
    index: Cache<String, u64>,
    max_bytes: u64,
    #[allow(dead_code)]
    pinned: Arc<Mutex<HashSet<String>>>,
}

impl LocalCdnCache {
    pub async fn new(cache_dir: PathBuf, max_bytes: u64) -> AppResult<Self> {
        fs::create_dir_all(&cache_dir).await?;
        Ok(Self {
            cache_dir,
            index: Cache::new(20_000),
            max_bytes,
            pinned: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    pub fn path_for_hash(&self, hash: &str) -> PathBuf {
        self.cache_dir.join(hash)
    }

    pub fn partial_path_for_hash(&self, hash: &str) -> PathBuf {
        self.cache_dir.join(format!("{hash}.partial"))
    }

    #[allow(dead_code)]
    pub fn pin(&self, hash: &str) -> AppResult<()> {
        let mut guard = self
            .pinned
            .lock()
            .map_err(|_| AppError::Concurrency("cache pin mutex poisoned".to_string()))?;
        guard.insert(hash.to_string());
        Ok(())
    }

    #[allow(dead_code)]
    pub fn unpin(&self, hash: &str) -> AppResult<()> {
        let mut guard = self
            .pinned
            .lock()
            .map_err(|_| AppError::Concurrency("cache pin mutex poisoned".to_string()))?;
        guard.remove(hash);
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn contains(&self, hash: &str) -> AppResult<bool> {
        if self.index.get(hash).await.is_some() {
            return Ok(true);
        }
        let path = self.path_for_hash(hash);
        Ok(fs::metadata(path).await.is_ok())
    }

    pub async fn read_chunk(&self, hash: &str) -> AppResult<Option<Vec<u8>>> {
        let path = self.path_for_hash(hash);
        if fs::metadata(&path).await.is_err() {
            return Ok(None);
        }
        let bytes = fs::read(&path).await?;
        self.index
            .insert(hash.to_string(), bytes.len() as u64)
            .await;
        Ok(Some(bytes))
    }

    pub async fn write_chunk(&self, hash: &str, bytes: &[u8]) -> AppResult<()> {
        let path = self.path_for_hash(hash);
        let partial = self.partial_path_for_hash(hash);
        fs::write(&partial, bytes).await?;
        if fs::metadata(&path).await.is_ok() {
            let _ = fs::remove_file(&path).await;
        }
        fs::rename(&partial, &path).await?;
        self.index
            .insert(hash.to_string(), bytes.len() as u64)
            .await;
        self.evict_if_needed().await
    }

    pub async fn import_file(&self, hash: &str, source_path: &Path) -> AppResult<()> {
        let path = self.path_for_hash(hash);
        let partial = self.partial_path_for_hash(hash);
        fs::copy(source_path, &partial).await?;
        if fs::metadata(&path).await.is_ok() {
            let _ = fs::remove_file(&path).await;
        }
        fs::rename(&partial, &path).await?;
        let size = fs::metadata(&path).await?.len();
        self.index.insert(hash.to_string(), size).await;
        self.evict_if_needed().await
    }

    pub async fn copy_to(&self, hash: &str, destination_path: &Path) -> AppResult<bool> {
        let path = self.path_for_hash(hash);
        if fs::metadata(&path).await.is_err() {
            return Ok(false);
        }
        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::copy(path, destination_path).await?;
        Ok(true)
    }

    async fn evict_if_needed(&self) -> AppResult<()> {
        let mut entries: Vec<(PathBuf, u64)> = Vec::new();
        let mut total = 0u64;
        let mut rd = fs::read_dir(&self.cache_dir).await?;
        while let Some(item) = rd.next_entry().await? {
            let meta = item.metadata().await?;
            if meta.is_file() {
                total += meta.len();
                entries.push((item.path(), meta.len()));
            }
        }

        if total <= self.max_bytes {
            return Ok(());
        }

        entries.sort_by_key(|(p, _)| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .ok()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        });

        let pinned = self
            .pinned
            .lock()
            .map_err(|_| AppError::Concurrency("cache pin mutex poisoned".to_string()))?
            .clone();

        for (path, size) in entries {
            if total <= self.max_bytes {
                break;
            }
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if pinned.contains(name) {
                    continue;
                }
            }
            if fs::remove_file(&path).await.is_ok() {
                total = total.saturating_sub(size);
            }
        }

        Ok(())
    }

    #[allow(dead_code)]
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hash_mapping() {
        let dir = tempfile::tempdir().unwrap();
        let cache = LocalCdnCache::new(dir.path().join("cache"), 1024)
            .await
            .unwrap();
        let path = cache.path_for_hash("abc123");
        assert!(path.ends_with("abc123"));
    }

    #[tokio::test]
    async fn partial_files_are_not_treated_as_cache_hits() {
        let dir = tempfile::tempdir().unwrap();
        let cache = LocalCdnCache::new(dir.path().join("cache"), 1024)
            .await
            .unwrap();
        let partial = cache.partial_path_for_hash("abc123");
        fs::write(&partial, b"incomplete").await.unwrap();

        assert!(!cache.contains("abc123").await.unwrap());
        assert!(cache.read_chunk("abc123").await.unwrap().is_none());
    }
}
