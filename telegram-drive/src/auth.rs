use crate::database::Database;
use crate::models::{AppError, AppResult, AuthState};
use crate::telegram::TelegramClient;
use aes_gcm::{aead::{Aead, KeyInit, OsRng}, AeadCore, Aes256Gcm, Nonce};
use sha2::{Digest, Sha256};

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
            let plain = decrypt_blob(&blob)?;
            let _ = self.telegram.restore_session_blob(&plain)?;
            let state = self.telegram.restore_runtime_auth().await?;
            return Ok(state);
        }
        Ok(AuthState::LoggedOut)
    }

    pub async fn start_phone_auth(&self, phone: String, api_id: i32, api_hash: String) -> AppResult<AuthState> {
        let state = self.telegram.start_phone_auth(phone, api_id, api_hash).await?;
        Ok(state)
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

    async fn persist_session(&self) -> AppResult<()> {
        let blob = self.telegram.session_blob()?;
        let encrypted = encrypt_blob(&blob)?;
        self.db.save_session_blob("primary", &encrypted)?;
        Ok(())
    }
}

fn machine_key() -> [u8; 32] {
    let seed = format!(
        "{}:{}",
        std::env::var("USERNAME").unwrap_or_else(|_| "user".to_string()),
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "host".to_string())
    );
    let mut hasher = Sha256::new();
    hasher.update(seed.as_bytes());
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest[..32]);
    out
}

fn encrypt_blob(plain: &[u8]) -> AppResult<Vec<u8>> {
    let key = machine_key();
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| AppError::Crypto(e.to_string()))?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let mut out = nonce.to_vec();
    let mut encrypted = cipher
        .encrypt(&nonce, plain)
        .map_err(|e| AppError::Crypto(e.to_string()))?;
    out.append(&mut encrypted);
    Ok(out)
}

fn decrypt_blob(cipher_text: &[u8]) -> AppResult<Vec<u8>> {
    if cipher_text.len() < 12 {
        return Err(AppError::Crypto("session blob too short".to_string()));
    }
    let (nonce, payload) = cipher_text.split_at(12);
    let key = machine_key();
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

        let s1 = auth
            .start_phone_auth("+551100000000".to_string(), 12345, "hash".to_string())
            .await
            .unwrap();
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
}
