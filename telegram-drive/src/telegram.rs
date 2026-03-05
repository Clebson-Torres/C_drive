use crate::models::{AppError, AppResult, AuthState};
use crate::session_store::PersistentSession;
use grammers_client::message::InputMessage;
use grammers_client::client::{LoginToken, PasswordToken};
use grammers_client::{Client, SignInError};
use grammers_mtsender::SenderPool;
use rand::Rng;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::fs;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tracing::warn;
use uuid::Uuid;

#[derive(Clone)]
pub struct TelegramClient {
    store_dir: PathBuf,
    metadata_dir: PathBuf,
    session_path: PathBuf,
    mock_mode: bool,
    auth: Arc<Mutex<AuthContext>>,
}

struct AuthContext {
    state: AuthState,
    api_id: Option<i32>,
    api_hash: Option<String>,
    phone: Option<String>,
    client: Option<Client>,
    runner_task: Option<JoinHandle<()>>,
    login_token: Option<LoginToken>,
    password_token: Option<PasswordToken>,
}

impl TelegramClient {
    pub async fn new(base_dir: PathBuf) -> AppResult<Self> {
        let mock_mode = std::env::var("TELEGRAM_DRIVE_AUTH_MOCK")
            .ok()
            .as_deref()
            == Some("1");
        Self::new_with_mode(base_dir, mock_mode).await
    }

    pub async fn new_with_mode(base_dir: PathBuf, mock_mode: bool) -> AppResult<Self> {
        let store_dir = base_dir.join("chunks");
        let metadata_dir = base_dir.join("metadata");
        fs::create_dir_all(&store_dir).await?;
        fs::create_dir_all(&metadata_dir).await?;

        Ok(Self {
            store_dir,
            metadata_dir,
            session_path: base_dir.join("telegram_session.bin"),
            mock_mode,
            auth: Arc::new(Mutex::new(AuthContext {
                state: AuthState::LoggedOut,
                api_id: None,
                api_hash: None,
                phone: None,
                client: None,
                runner_task: None,
                login_token: None,
                password_token: None,
            })),
        })
    }

    pub fn auth_state(&self) -> AuthState {
        self.auth
            .lock()
            .map(|a| a.state.clone())
            .unwrap_or(AuthState::LoggedOut)
    }

    pub fn is_logged_in(&self) -> bool {
        matches!(self.auth_state(), AuthState::LoggedIn)
    }

    pub async fn start_phone_auth(&self, phone: String, api_id: i32, api_hash: String) -> AppResult<AuthState> {
        if self.mock_mode {
            let mut auth = self.auth_lock()?;
            auth.phone = Some(phone);
            auth.api_id = Some(api_id);
            auth.api_hash = Some(api_hash);
            auth.state = AuthState::AwaitingCode;
            return Ok(auth.state.clone());
        }

        let client = self.ensure_client(api_id).await?;

        if client
            .is_authorized()
            .await
            .map_err(|e| AppError::Telegram(format!("is_authorized failed: {e}")))?
        {
            let mut auth = self.auth_lock()?;
            auth.state = AuthState::LoggedIn;
            auth.phone = Some(phone);
            auth.api_id = Some(api_id);
            auth.api_hash = Some(api_hash);
            auth.login_token = None;
            auth.password_token = None;
            return Ok(auth.state.clone());
        }

        let token = client
            .request_login_code(&phone, &api_hash)
            .await
            .map_err(|e| AppError::Telegram(format!("request_login_code failed: {e}")))?;

        let mut auth = self.auth_lock()?;
        auth.phone = Some(phone);
        auth.api_id = Some(api_id);
        auth.api_hash = Some(api_hash);
        auth.login_token = Some(token);
        auth.password_token = None;
        auth.state = AuthState::AwaitingCode;
        Ok(auth.state.clone())
    }

    pub async fn verify_code(&self, code: String) -> AppResult<AuthState> {
        if self.mock_mode {
            let mut auth = self.auth_lock()?;
            match code.trim() {
                "12345" => {
                    auth.state = AuthState::LoggedIn;
                    return Ok(AuthState::LoggedIn);
                }
                "00000" => {
                    auth.state = AuthState::AwaitingPassword;
                    return Ok(AuthState::AwaitingPassword);
                }
                _ => return Err(AppError::Validation("invalid login code".to_string())),
            }
        }

        let (client, token) = {
            let mut auth = self.auth_lock()?;
            let client = auth
                .client
                .clone()
                .ok_or_else(|| AppError::Validation("telegram client not initialized".to_string()))?;
            let token = auth
                .login_token
                .take()
                .ok_or_else(|| AppError::Validation("missing login token; start auth first".to_string()))?;
            (client, token)
        };

        match client.sign_in(&token, code.trim()).await {
            Ok(_) => {
                let mut auth = self.auth_lock()?;
                auth.state = AuthState::LoggedIn;
                auth.password_token = None;
                Ok(AuthState::LoggedIn)
            }
            Err(SignInError::PasswordRequired(password_token)) => {
                let mut auth = self.auth_lock()?;
                auth.password_token = Some(password_token);
                auth.state = AuthState::AwaitingPassword;
                Ok(AuthState::AwaitingPassword)
            }
            Err(SignInError::InvalidCode) => Err(AppError::Validation("invalid login code".to_string())),
            Err(e) => Err(AppError::Telegram(format!("sign_in failed: {e}"))),
        }
    }

    pub async fn verify_password(&self, password: String) -> AppResult<AuthState> {
        if self.mock_mode {
            let mut auth = self.auth_lock()?;
            if password.trim() == "password123" {
                auth.state = AuthState::LoggedIn;
                return Ok(AuthState::LoggedIn);
            }
            auth.state = AuthState::AwaitingPassword;
            return Err(AppError::Validation("invalid 2FA password".to_string()));
        }

        let (client, password_token) = {
            let mut auth = self.auth_lock()?;
            let client = auth
                .client
                .clone()
                .ok_or_else(|| AppError::Validation("telegram client not initialized".to_string()))?;
            let token = auth
                .password_token
                .take()
                .ok_or_else(|| AppError::Validation("2FA password token not available".to_string()))?;
            (client, token)
        };

        match client.check_password(password_token, password.as_bytes()).await {
            Ok(_) => {
                let mut auth = self.auth_lock()?;
                auth.state = AuthState::LoggedIn;
                Ok(AuthState::LoggedIn)
            }
            Err(SignInError::InvalidPassword(token)) => {
                let mut auth = self.auth_lock()?;
                auth.password_token = Some(token);
                auth.state = AuthState::AwaitingPassword;
                Err(AppError::Validation("invalid 2FA password".to_string()))
            }
            Err(e) => Err(AppError::Telegram(format!("check_password failed: {e}"))),
        }
    }

    pub async fn restore_runtime_auth(&self) -> AppResult<AuthState> {
        if self.mock_mode {
            return Ok(self.auth_state());
        }
        let (api_id, has_creds) = {
            let auth = self.auth_lock()?;
            (auth.api_id, auth.api_hash.is_some())
        };
        let api_id = match api_id {
            Some(v) if has_creds => v,
            _ => {
                let mut auth = self.auth_lock()?;
                auth.state = AuthState::LoggedOut;
                return Ok(AuthState::LoggedOut);
            }
        };

        let client = self.ensure_client(api_id).await?;
        let authorized = client
            .is_authorized()
            .await
            .map_err(|e| AppError::Telegram(format!("is_authorized failed on restore: {e}")))?;
        let mut auth = self.auth_lock()?;
        auth.state = if authorized {
            AuthState::LoggedIn
        } else {
            AuthState::LoggedOut
        };
        Ok(auth.state.clone())
    }

    pub async fn upload_chunk(&self, payload: Vec<u8>) -> AppResult<String> {
        self.ensure_auth()?;
        if self.mock_mode {
            return self
                .with_retry(|| async {
                    let id = Uuid::new_v4().to_string();
                    let path = self.store_dir.join(&id);
                    fs::write(path, payload.clone()).await?;
                    Ok(id)
                })
                .await;
        }

        let api_id = self
            .auth_lock()?
            .api_id
            .ok_or_else(|| AppError::Telegram("missing api_id in auth context".to_string()))?;
        let client = self.ensure_client(api_id).await?;
        let peer = self.saved_messages_peer(&client).await?;

        let temp_file = self.store_dir.join(format!("upload-{}.bin", Uuid::new_v4()));
        fs::write(&temp_file, &payload).await?;
        let result = self
            .with_retry(|| {
                let client = client.clone();
                let temp_file = temp_file.clone();
                async move {
                    let uploaded = client.upload_file(&temp_file).await.map_err(|e| {
                        AppError::Telegram(format!("telegram upload_file failed: {e}"))
                    })?;
                    let message = client
                        .send_message(
                            peer,
                            InputMessage::new().text("tgdrive-chunk").file(uploaded),
                        )
                        .await
                        .map_err(|e| {
                            AppError::Telegram(format!("telegram send_message failed: {e}"))
                        })?;
                    Ok(message.id().to_string())
                }
            })
            .await;
        let _ = fs::remove_file(&temp_file).await;
        result
    }

    pub async fn download_chunk(&self, telegram_file_id: &str) -> AppResult<Vec<u8>> {
        self.ensure_auth()?;
        if self.mock_mode {
            let id = telegram_file_id.to_string();
            return self
                .with_retry(|| async {
                    let path = self.store_dir.join(&id);
                    let bytes = fs::read(path).await?;
                    Ok(bytes)
                })
                .await;
        }

        let message_id: i32 = telegram_file_id.parse().map_err(|_| {
            AppError::Validation(format!(
                "invalid telegram_file_id (expected message id): {telegram_file_id}"
            ))
        })?;
        let api_id = self
            .auth_lock()?
            .api_id
            .ok_or_else(|| AppError::Telegram("missing api_id in auth context".to_string()))?;
        let client = self.ensure_client(api_id).await?;
        let peer = self.saved_messages_peer(&client).await?;

        let temp_file = self.store_dir.join(format!("download-{}.bin", Uuid::new_v4()));
        let result = self
            .with_retry(|| {
                let client = client.clone();
                let temp_file = temp_file.clone();
                async move {
                    let messages = client
                        .get_messages_by_id(peer, &[message_id])
                        .await
                        .map_err(|e| {
                            AppError::Telegram(format!(
                                "telegram get_messages_by_id failed: {e}"
                            ))
                        })?;
                    let message = messages
                        .into_iter()
                        .next()
                        .flatten()
                        .ok_or_else(|| AppError::Telegram("chunk message not found".to_string()))?;
                    let media = message.media().ok_or_else(|| {
                        AppError::Telegram("chunk message has no downloadable media".to_string())
                    })?;
                    client.download_media(&media, &temp_file).await?;
                    Ok(fs::read(&temp_file).await?)
                }
            })
            .await;
        let _ = fs::remove_file(&temp_file).await;
        result
    }

    pub async fn backup_metadata_snapshot(&self, encrypted_snapshot: &[u8]) -> AppResult<String> {
        self.ensure_auth()?;
        let id = format!("meta-{}", Uuid::new_v4());
        fs::write(self.metadata_dir.join(&id), encrypted_snapshot).await?;
        Ok(id)
    }

    pub async fn latest_metadata_snapshot(&self) -> AppResult<Option<Vec<u8>>> {
        self.ensure_auth()?;
        let mut entries = fs::read_dir(&self.metadata_dir).await?;
        let mut latest: Option<(std::time::SystemTime, PathBuf)> = None;

        while let Some(e) = entries.next_entry().await? {
            let meta = e.metadata().await?;
            let modified = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            match latest {
                Some((ts, _)) if modified <= ts => {}
                _ => latest = Some((modified, e.path())),
            }
        }

        if let Some((_, path)) = latest {
            return Ok(Some(fs::read(path).await?));
        }
        Ok(None)
    }

    pub fn session_blob(&self) -> AppResult<Vec<u8>> {
        let auth = self.auth_lock()?;
        let payload = serde_json::json!({
            "state": auth.state,
            "api_id": auth.api_id,
            "api_hash": auth.api_hash,
            "phone": auth.phone
        });
        Ok(serde_json::to_vec(&payload)?)
    }

    pub fn restore_session_blob(&self, blob: &[u8]) -> AppResult<AuthState> {
        let value: serde_json::Value = serde_json::from_slice(blob)?;
        let state: AuthState = serde_json::from_value(value["state"].clone())
            .map_err(|e| AppError::Validation(format!("invalid stored auth state: {e}")))?;

        let mut auth = self.auth_lock()?;
        auth.state = state.clone();
        auth.api_id = value["api_id"].as_i64().map(|v| v as i32);
        auth.api_hash = value["api_hash"].as_str().map(ToOwned::to_owned);
        auth.phone = value["phone"].as_str().map(ToOwned::to_owned);
        Ok(auth.state.clone())
    }

    fn ensure_auth(&self) -> AppResult<()> {
        if !self.is_logged_in() {
            return Err(AppError::Telegram("not authenticated".to_string()));
        }
        Ok(())
    }

    async fn saved_messages_peer(
        &self,
        client: &Client,
    ) -> AppResult<grammers_session::types::PeerRef> {
        let me = client
            .get_me()
            .await
            .map_err(|e| AppError::Telegram(format!("get_me failed: {e}")))?;
        me.to_ref()
            .await
            .ok_or_else(|| AppError::Telegram("unable to resolve self peer".to_string()))
    }

    async fn ensure_client(&self, api_id: i32) -> AppResult<Client> {
        if let Some(client) = self.auth_lock()?.client.clone() {
            return Ok(client);
        }

        let session = Arc::new(PersistentSession::open_or_create(&self.session_path));

        let sender_pool = SenderPool::new(session, api_id);
        let client = Client::new(sender_pool.handle.clone());
        let runner_task = tokio::spawn(async move {
            let _ = sender_pool.runner.run().await;
        });

        let mut auth = self.auth_lock()?;
        auth.client = Some(client.clone());
        auth.runner_task = Some(runner_task);
        Ok(client)
    }

    fn auth_lock(&self) -> AppResult<std::sync::MutexGuard<'_, AuthContext>> {
        self.auth
            .lock()
            .map_err(|_| AppError::Concurrency("auth mutex poisoned".to_string()))
    }

    async fn with_retry<F, Fut, T>(&self, mut op: F) -> AppResult<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = AppResult<T>>,
    {
        let mut attempt = 0u32;
        let mut delay = 100u64;
        loop {
            match op().await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    attempt += 1;
                    if attempt >= 4 {
                        return Err(e);
                    }
                    let jitter = rand::thread_rng().gen_range(0..60u64);
                    warn!(attempt, error = %e, "telegram operation failed, retrying");
                    sleep(Duration::from_millis(delay + jitter)).await;
                    delay = (delay * 2).min(2_000);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn mock_auth_requires_password_when_code_indicates_2fa() {
        let temp = tempdir().unwrap();
        let tg = TelegramClient::new_with_mode(temp.path().join("tg"), true)
            .await
            .unwrap();
        let state = tg
            .start_phone_auth("+551100000000".to_string(), 10, "abc".to_string())
            .await
            .unwrap();
        assert!(matches!(state, AuthState::AwaitingCode));

        let state = tg.verify_code("00000".to_string()).await.unwrap();
        assert!(matches!(state, AuthState::AwaitingPassword));

        let err = tg.verify_password("wrong".to_string()).await.unwrap_err();
        assert!(err.to_string().contains("invalid 2FA password"));

        let state = tg.verify_password("password123".to_string()).await.unwrap();
        assert!(matches!(state, AuthState::LoggedIn));
    }
}
