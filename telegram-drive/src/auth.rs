use crate::database::{Database, NewFileRecord};
use crate::models::{
    AppError, AppResult, AuthPrefillDto, AuthState, FileOrigin, StorageMode, UserProfileDto,
};
use crate::security::derive_local_key;
use crate::telegram::TelegramClient;
use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    AeadCore, Aes256Gcm, Nonce,
};

#[derive(Clone)]
pub struct AuthService {
    db: Database,
    telegram: TelegramClient,
}

impl AuthService {
    pub fn new(db: Database, telegram: TelegramClient) -> Self {
        Self { db, telegram }
    }

    #[tracing::instrument(skip(self), name = "session_restore")]
    pub async fn restore_session(&self) -> AppResult<AuthState> {
        if let Some(blob) = self.db.load_session_blob("primary")? {
            let plain = decrypt_blob(self.session_salt_path(), &blob)?;
            let _ = self.telegram.restore_session_blob(&plain)?;
            let state = self.telegram.restore_runtime_auth().await?;
            return Ok(state);
        }
        Ok(AuthState::LoggedOut)
    }

    pub async fn start_phone_auth(&self, phone: String) -> AppResult<AuthState> {
        self.db
            .set_setting_json("auth.prefill", &AuthPrefillDto { phone: phone.clone() })?;
        self.telegram.start_phone_auth(phone).await
    }

    pub async fn verify_code(&self, code: String) -> AppResult<AuthState> {
        let state = self.telegram.verify_code(code).await?;
        self.persist_session().await?;
        Ok(state)
    }

    pub async fn verify_password(&self, password: String) -> AppResult<AuthState> {
        let state = self.telegram.verify_password(password).await?;
        self.persist_session().await?;
        Ok(state)
    }

    pub fn status(&self) -> AuthState {
        self.telegram.auth_state()
    }

    pub async fn profile(&self) -> AppResult<UserProfileDto> {
        self.telegram.profile().await
    }

    pub async fn logout(&self) -> AppResult<AuthState> {
        self.telegram.logout().await?;
        self.db.delete_session_blob("primary")?;
        Ok(AuthState::LoggedOut)
    }

    pub fn prefill(&self) -> AppResult<Option<AuthPrefillDto>> {
        self.db.get_setting_json("auth.prefill")
    }

    pub async fn sync_saved_messages_index(&self) -> AppResult<usize> {
        let root_folder_id = self.db.root_folder_id()?;
        let imported = self.telegram.list_saved_message_files().await?;
        let mut synced = 0usize;

        for file in imported {
            self.db.upsert_imported_file(
                NewFileRecord {
                    name: file.name,
                    size: file.size,
                    hash: format!("telegram-import:{}", file.telegram_file_id),
                    folder_id: root_folder_id,
                    mime_type: file.mime_type,
                    original_path: None,
                    storage_mode: StorageMode::Single,
                    telegram_file_id: Some(file.telegram_file_id),
                    origin: FileOrigin::Imported,
                },
                file.created_at,
                file.created_at,
            )?;
            synced += 1;
        }

        Ok(synced)
    }

    async fn persist_session(&self) -> AppResult<()> {
        let blob = self.telegram.session_blob()?;
        let encrypted = encrypt_blob(self.session_salt_path(), &blob)?;
        self.db.save_session_blob("primary", &encrypted)?;
        Ok(())
    }

    fn session_salt_path(&self) -> std::path::PathBuf {
        self.db.app_dir().join("auth-session.salt")
    }
}

fn encrypt_blob(
    salt_path: impl AsRef<std::path::Path>,
    plain: &[u8],
) -> AppResult<Vec<u8>> {
    let key = derive_local_key(salt_path.as_ref(), "auth-session-blob")?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| AppError::Crypto(e.to_string()))?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let mut out = nonce.to_vec();
    let mut encrypted = cipher
        .encrypt(&nonce, plain)
        .map_err(|e| AppError::Crypto(e.to_string()))?;
    out.append(&mut encrypted);
    Ok(out)
}

fn decrypt_blob(
    salt_path: impl AsRef<std::path::Path>,
    cipher_text: &[u8],
) -> AppResult<Vec<u8>> {
    if cipher_text.len() < 12 {
        return Err(AppError::Crypto("session blob too short".to_string()));
    }
    let (nonce, payload) = cipher_text.split_at(12);
    let key = derive_local_key(salt_path.as_ref(), "auth-session-blob")?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| AppError::Crypto(e.to_string()))?;
    cipher
        .decrypt(Nonce::from_slice(nonce), payload)
        .map_err(|e| AppError::Crypto(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn auth_flow_and_restore_with_mock_backend() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("test.db");
        let db = Database::open(&db_path).unwrap();
        let tg = TelegramClient::new_with_mode(temp.path().join("tg"), true)
            .await
            .unwrap();
        let auth = AuthService::new(db.clone(), tg);

        let s1 = auth.start_phone_auth("+551100000000".to_string()).await.unwrap();
        assert!(matches!(s1, AuthState::AwaitingCode));

        let s2 = auth.verify_code("12345".to_string()).await.unwrap();
        assert!(matches!(s2, AuthState::LoggedIn));

        let tg2 = TelegramClient::new_with_mode(temp.path().join("tg2"), true)
            .await
            .unwrap();
        let auth2 = AuthService::new(db, tg2);
        let restored = auth2.restore_session().await.unwrap();
        assert!(matches!(restored, AuthState::LoggedIn));
    }

    #[tokio::test]
    async fn logout_keeps_phone_prefill_and_clears_session_restore() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("test.db");
        let db = Database::open(&db_path).unwrap();
        let tg = TelegramClient::new_with_mode(temp.path().join("tg"), true)
            .await
            .unwrap();
        let auth = AuthService::new(db.clone(), tg);

        auth.start_phone_auth("+5511999999999".to_string())
            .await
            .unwrap();
        auth.verify_code("12345".to_string()).await.unwrap();

        let prefill = auth.prefill().unwrap().unwrap();
        assert_eq!(prefill.phone, "+5511999999999");
        let serialized = serde_json::to_string(&prefill).unwrap();
        assert!(!serialized.contains("api_hash"));

        let logged_out = auth.logout().await.unwrap();
        assert!(matches!(logged_out, AuthState::LoggedOut));

        let tg2 = TelegramClient::new_with_mode(temp.path().join("tg2"), true)
            .await
            .unwrap();
        let auth2 = AuthService::new(db, tg2);
        let restored = auth2.restore_session().await.unwrap();
        assert!(matches!(restored, AuthState::LoggedOut));

        let prefill_after = auth2.prefill().unwrap().unwrap();
        assert_eq!(prefill_after.phone, "+5511999999999");
    }
}
