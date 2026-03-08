use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    AeadCore, Aes256Gcm, Nonce,
};
use crate::security::{derive_legacy_local_key, derive_local_key};
use futures_core::future::BoxFuture;
use grammers_session::types::{DcOption, PeerId, PeerInfo, UpdateState, UpdatesState};
use grammers_session::{Session, SessionData};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{sleep, Duration};
use tracing::{error, warn};

#[derive(Clone, Serialize, Deserialize)]
struct PersistedState {
    home_dc: i32,
    dc_options: HashMap<i32, DcOption>,
    peer_infos: HashMap<PeerId, PeerInfo>,
    updates_state: UpdatesState,
}

impl Default for PersistedState {
    fn default() -> Self {
        let base = SessionData::default();
        Self {
            home_dc: base.home_dc,
            dc_options: base.dc_options,
            peer_infos: base.peer_infos,
            updates_state: base.updates_state,
        }
    }
}

pub struct PersistentSession {
    #[allow(dead_code)]
    path: PathBuf,
    #[allow(dead_code)]
    key: [u8; 32],
    cache: Arc<RwLock<LoadedState>>,
    save_tx: mpsc::UnboundedSender<()>,
}

impl PersistentSession {
    pub fn open_or_create(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        let key = derive_local_key(&salt_path_for(&path), "telegram-session-store")
            .map_err(|e| e.to_string())?;
        let loaded = load_state_or_default(&path, &key, &salt_path_for(&path));
        let cache = Arc::new(RwLock::new(loaded));
        let (save_tx, mut save_rx) = mpsc::unbounded_channel::<()>();

        let path_clone = path.clone();
        let key_clone = key;
        let cache_clone = Arc::clone(&cache);
        tokio::spawn(async move {
            while save_rx.recv().await.is_some() {
                sleep(Duration::from_millis(150)).await;
                while save_rx.try_recv().is_ok() {}

                let snapshot = cache_clone.read().await.clone();
                if let Err(e) = save_snapshot(&path_clone, &key_clone, &snapshot.state).await {
                    error!(error = %e, "failed to persist telegram session snapshot");
                }
            }
        });

        let session = Self {
            path,
            key,
            cache,
            save_tx,
        };

        if session.cache.try_read().map(|state| state.needs_rewrite).unwrap_or(false) {
            session.schedule_save();
        }

        Ok(session)
    }

    fn schedule_save(&self) {
        let _ = self.save_tx.send(());
    }

    #[cfg(test)]
    pub async fn flush_now(&self) {
        let snapshot = self.cache.read().await.clone();
        if let Err(e) = save_snapshot(&self.path, &self.key, &snapshot.state).await {
            error!(error = %e, "failed to flush telegram session snapshot");
        }
    }
}

#[derive(Clone)]
struct LoadedState {
    state: PersistedState,
    needs_rewrite: bool,
}

impl Session for PersistentSession {
    fn home_dc_id(&self) -> i32 {
        if let Ok(guard) = self.cache.try_read() {
            guard.state.home_dc
        } else {
            SessionData::default().home_dc
        }
    }

    fn set_home_dc_id(&self, dc_id: i32) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.cache.write().await.state.home_dc = dc_id;
            self.schedule_save();
        })
    }

    fn dc_option(&self, dc_id: i32) -> Option<DcOption> {
        if let Ok(guard) = self.cache.try_read() {
            guard.state.dc_options.get(&dc_id).cloned()
        } else {
            None
        }
    }

    fn set_dc_option(&self, dc_option: &DcOption) -> BoxFuture<'_, ()> {
        let dc_option = dc_option.clone();
        Box::pin(async move {
            self.cache
                .write()
                .await
                .state
                .dc_options
                .insert(dc_option.id, dc_option);
            self.schedule_save();
        })
    }

    fn peer(&self, peer: PeerId) -> BoxFuture<'_, Option<PeerInfo>> {
        Box::pin(async move { self.cache.read().await.state.peer_infos.get(&peer).cloned() })
    }

    fn cache_peer(&self, peer: &PeerInfo) -> BoxFuture<'_, ()> {
        let peer = peer.clone();
        Box::pin(async move {
            self.cache.write().await.state.peer_infos.insert(peer.id(), peer);
            self.schedule_save();
        })
    }

    fn updates_state(&self) -> BoxFuture<'_, UpdatesState> {
        Box::pin(async move { self.cache.read().await.state.updates_state.clone() })
    }

    fn set_update_state(&self, update: UpdateState) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let mut guard = self.cache.write().await;
            match update {
                UpdateState::All(state) => guard.state.updates_state = state,
                UpdateState::Primary { pts, date, seq } => {
                    guard.state.updates_state.pts = pts;
                    guard.state.updates_state.date = date;
                    guard.state.updates_state.seq = seq;
                }
                UpdateState::Secondary { qts } => {
                    guard.state.updates_state.qts = qts;
                }
                UpdateState::Channel { id, pts } => {
                    guard.state.updates_state.channels.retain(|c| c.id != id);
                    guard
                        .state
                        .updates_state
                        .channels
                        .push(grammers_session::types::ChannelState { id, pts });
                }
            }
            self.schedule_save();
        })
    }
}

fn salt_path_for(path: &Path) -> PathBuf {
    path.with_extension("salt")
}

fn load_state_or_default(path: &Path, key: &[u8; 32], salt_path: &Path) -> LoadedState {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => {
            return LoadedState {
                state: PersistedState::default(),
                needs_rewrite: false,
            }
        }
    };

    match decrypt_blob(key, &bytes)
        .map(|plain| (plain, false))
        .or_else(|_| {
            let legacy_key = derive_legacy_local_key(salt_path, "telegram-session-store")
                .map_err(|e| e.to_string())?;
            decrypt_blob(&legacy_key, &bytes).map(|plain| (plain, true))
        })
        .and_then(|(plain, migrated)| {
            serde_json::from_slice::<PersistedState>(&plain)
                .map_err(|e| e.to_string())
                .map(|state| LoadedState {
                    state,
                    needs_rewrite: migrated,
                })
        })
    {
        Ok(state) => state,
        Err(e) => {
            warn!(error = %e, path = %path.display(), "corrupted telegram session snapshot; backing up and resetting");
            if let Some(parent) = path.parent() {
                let ts = chrono::Utc::now().timestamp();
                let backup = parent.join(format!("telegram_session.corrupt-{ts}.bin"));
                let _ = std::fs::rename(path, backup);
            }
            LoadedState {
                state: PersistedState::default(),
                needs_rewrite: false,
            }
        }
    }
}

async fn save_snapshot(
    path: &Path,
    key: &[u8; 32],
    snapshot: &PersistedState,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let plain = serde_json::to_vec(snapshot).map_err(|e| e.to_string())?;
    let encrypted = encrypt_blob(key, &plain)?;

    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, encrypted)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn encrypt_blob(key: &[u8; 32], plain: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let mut out = nonce.to_vec();
    let mut encrypted = cipher.encrypt(&nonce, plain).map_err(|e| e.to_string())?;
    out.append(&mut encrypted);
    Ok(out)
}

fn decrypt_blob(key: &[u8; 32], cipher_text: &[u8]) -> Result<Vec<u8>, String> {
    if cipher_text.len() < 12 {
        return Err("session blob too short".to_string());
    }
    let (nonce, payload) = cipher_text.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    cipher
        .decrypt(Nonce::from_slice(nonce), payload)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::derive_legacy_local_key;

    #[tokio::test]
    async fn snapshot_roundtrip_and_reload() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("telegram_session.bin");

        let session = PersistentSession::open_or_create(&path);
        let session = session.unwrap();
        session.set_home_dc_id(4).await;
        session.flush_now().await;

        let reloaded = PersistentSession::open_or_create(&path);
        assert_eq!(reloaded.unwrap().home_dc_id(), 4);
    }

    #[tokio::test]
    async fn corrupted_file_is_backed_up() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("telegram_session.bin");
        tokio::fs::write(&path, b"invalid").await.unwrap();

        let _ = PersistentSession::open_or_create(&path);
        assert!(!tokio::fs::metadata(&path).await.is_ok());

        let mut found_backup = false;
        let mut rd = tokio::fs::read_dir(temp.path()).await.unwrap();
        while let Some(entry) = rd.next_entry().await.unwrap() {
            if entry
                .file_name()
                .to_string_lossy()
                .contains("telegram_session.corrupt-")
            {
                found_backup = true;
                break;
            }
        }
        assert!(found_backup);
    }

    #[tokio::test]
    async fn legacy_snapshot_is_rewritten_with_platform_secret() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("telegram_session.bin");
        let salt_path = salt_path_for(&path);

        let legacy_state = PersistedState {
            home_dc: 9,
            ..PersistedState::default()
        };
        let legacy_key = derive_legacy_local_key(&salt_path, "telegram-session-store").unwrap();
        let legacy_bytes = encrypt_blob(&legacy_key, &serde_json::to_vec(&legacy_state).unwrap()).unwrap();
        tokio::fs::write(&path, legacy_bytes.clone()).await.unwrap();

        let session = PersistentSession::open_or_create(&path).unwrap();
        assert_eq!(session.home_dc_id(), 9);
        session.flush_now().await;

        let rewritten = tokio::fs::read(&path).await.unwrap();
        assert_ne!(rewritten, legacy_bytes);
        let current_key = derive_local_key(&salt_path, "telegram-session-store").unwrap();
        let plain = decrypt_blob(&current_key, &rewritten).unwrap();
        let state: PersistedState = serde_json::from_slice(&plain).unwrap();
        assert_eq!(state.home_dc, 9);
    }
}
