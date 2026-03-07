use crate::models::{AppError, AppResult, AuthState, UserProfileDto};
use crate::session_store::PersistentSession;
use grammers_client::client::{LoginToken, PasswordToken};
use grammers_client::message::InputMessage;
use grammers_client::peer::Peer;
use grammers_client::{Client, SignInError};
use grammers_mtsender::SenderPool;
use rand::Rng;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::fs;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tracing::warn;
use uuid::Uuid;

// Quantos clientes TCP paralelos manter para uploads.
// Cada um tem seu próprio SenderPool → conexão MTProto independente.
// 4 é conservador; pode subir para 8 se não houver flood errors.
const UPLOAD_POOL_SIZE: usize = 4;

#[derive(Clone)]
pub struct TelegramClient {
    store_dir: PathBuf,
    #[allow(dead_code)]
    metadata_dir: PathBuf,
    session_path: PathBuf,
    mock_mode: bool,
    auth: Arc<Mutex<AuthContext>>,
    // Pool de clientes dedicados para upload — round-robin atômico
    upload_pool: Arc<UploadClientPool>,
}

// ── Pool interno ─────────────────────────────────────────────────────────────

struct UploadClientPool {
    clients: Mutex<Vec<Client>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    counter: AtomicUsize,
}

impl UploadClientPool {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            clients: Mutex::new(Vec::new()),
            tasks: Mutex::new(Vec::new()),
            counter: AtomicUsize::new(0),
        })
    }

    /// Inicializa N clientes compartilhando a mesma sessão já autenticada.
    /// Chamado uma única vez após login bem-sucedido.
    async fn init(&self, session_path: &Path, api_id: i32, n: usize) -> AppResult<()> {
        let mut clients = self
            .clients
            .lock()
            .map_err(|_| AppError::Concurrency("upload pool mutex poisoned".to_string()))?;
        let mut tasks = self
            .tasks
            .lock()
            .map_err(|_| AppError::Concurrency("upload pool tasks mutex poisoned".to_string()))?;

        // Limpa pool anterior se existir (ex: re-login)
        for t in tasks.drain(..) {
            t.abort();
        }
        clients.clear();

        for i in 0..n {
            // Cada cliente lê a mesma sessão em disco — já tem auth_key
            let session = Arc::new(PersistentSession::open_or_create(session_path));
            let sender_pool = SenderPool::new(session, api_id);
            let client = Client::new(sender_pool.handle.clone());
            let task = tokio::spawn(async move {
                let _ = sender_pool.runner.run().await;
                warn!(pool_index = i, "upload pool runner exited");
            });
            clients.push(client);
            tasks.push(task);
        }

        Ok(())
    }

    /// Retorna o próximo cliente via round-robin (sem lock, atômico).
    fn next(&self) -> AppResult<Client> {
        let clients = self
            .clients
            .lock()
            .map_err(|_| AppError::Concurrency("upload pool mutex poisoned".to_string()))?;
        if clients.is_empty() {
            return Err(AppError::Telegram(
                "upload pool não inicializado — faça login primeiro".to_string(),
            ));
        }
        let idx = self.counter.fetch_add(1, Ordering::Relaxed) % clients.len();
        Ok(clients[idx].clone())
    }

    fn shutdown(&self) {
        if let Ok(mut tasks) = self.tasks.lock() {
            for t in tasks.drain(..) {
                t.abort();
            }
        }
        if let Ok(mut clients) = self.clients.lock() {
            clients.clear();
        }
    }
}

// ── AuthContext (sem mudança estrutural) ─────────────────────────────────────

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

// ── impl TelegramClient ───────────────────────────────────────────────────────

impl TelegramClient {
    pub async fn new(base_dir: PathBuf) -> AppResult<Self> {
        let mock_mode = std::env::var("TGDRIVE_MOCK")
            .or_else(|_| std::env::var("TELEGRAM_DRIVE_AUTH_MOCK"))
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
            upload_pool: UploadClientPool::new(),
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

    fn auth_lock(&self) -> AppResult<std::sync::MutexGuard<'_, AuthContext>> {
        self.auth
            .lock()
            .map_err(|_| AppError::Concurrency("auth mutex poisoned".to_string()))
    }

    pub async fn start_phone_auth(
        &self,
        phone: String,
        api_id: i32,
        api_hash: String,
    ) -> AppResult<AuthState> {
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
            // Extrai api_id antes de qualquer await para não segurar MutexGuard
            let api_id_val = {
                let mut auth = self.auth_lock()?;
                auth.state = AuthState::LoggedIn;
                auth.phone = Some(phone);
                auth.api_id = Some(api_id);
                auth.api_hash = Some(api_hash);
                auth.login_token = None;
                auth.password_token = None;
                api_id
            };
            self.init_upload_pool(api_id_val).await?;
            return Ok(AuthState::LoggedIn);
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
            let client = auth.client.clone().ok_or_else(|| {
                AppError::Validation("telegram client not initialized".to_string())
            })?;
            let token = auth.login_token.take().ok_or_else(|| {
                AppError::Validation("missing login token; start auth first".to_string())
            })?;
            (client, token)
        };

        match client.sign_in(&token, code.trim()).await {
            Ok(_) => {
                let api_id = {
                    let mut auth = self.auth_lock()?;
                    auth.state = AuthState::LoggedIn;
                    auth.password_token = None;
                    auth.api_id
                };
                if let Some(id) = api_id {
                    self.init_upload_pool(id).await?;
                }
                Ok(AuthState::LoggedIn)
            }
            Err(SignInError::PasswordRequired(password_token)) => {
                let mut auth = self.auth_lock()?;
                auth.state = AuthState::AwaitingPassword;
                auth.password_token = Some(password_token);
                Ok(AuthState::AwaitingPassword)
            }
            Err(e) => Err(AppError::Telegram(format!("sign_in failed: {e}"))),
        }
    }

    pub async fn verify_password(&self, password: String) -> AppResult<AuthState> {
        if self.mock_mode {
            let mut auth = self.auth_lock()?;
            if password == "password123" {
                auth.state = AuthState::LoggedIn;
                return Ok(AuthState::LoggedIn);
            }
            return Err(AppError::Validation("invalid 2FA password".to_string()));
        }

        let (client, token) = {
            let mut auth = self.auth_lock()?;
            let client = auth.client.clone().ok_or_else(|| {
                AppError::Validation("telegram client not initialized".to_string())
            })?;
            let token = auth
                .password_token
                .take()
                .ok_or_else(|| AppError::Validation("missing password token".to_string()))?;
            (client, token)
        };

        client
            .check_password(token, password.trim())
            .await
            .map_err(|e| AppError::Telegram(format!("check_password failed: {e}")))?;

        let api_id = {
            let mut auth = self.auth_lock()?;
            auth.state = AuthState::LoggedIn;
            auth.api_id
        };
        if let Some(id) = api_id {
            self.init_upload_pool(id).await?;
        }
        Ok(AuthState::LoggedIn)
    }

    pub async fn restore_session(&self) -> AppResult<AuthState> {
        if self.mock_mode {
            return Ok(self.auth_state());
        }

        let api_id = match self.auth_lock()?.api_id {
            Some(id) => id,
            None => return Ok(AuthState::LoggedOut),
        };

        let client = self.ensure_client(api_id).await?;
        if client
            .is_authorized()
            .await
            .map_err(|e| AppError::Telegram(format!("is_authorized failed: {e}")))?
        {
            let mut auth = self.auth_lock()?;
            auth.state = AuthState::LoggedIn;
            drop(auth);
            self.init_upload_pool(api_id).await?;
            return Ok(AuthState::LoggedIn);
        }

        Ok(AuthState::LoggedOut)
    }

    /// Inicializa o pool de clientes de upload após autenticação.
    async fn init_upload_pool(&self, api_id: i32) -> AppResult<()> {
        if self.mock_mode {
            return Ok(());
        }
        self.upload_pool
            .init(&self.session_path, api_id, UPLOAD_POOL_SIZE)
            .await
    }

    pub async fn profile(&self) -> AppResult<UserProfileDto> {
        self.ensure_auth()?;
        let api_id = self
            .auth_lock()?
            .api_id
            .ok_or_else(|| AppError::Telegram("missing api_id".to_string()))?;
        let client = self.ensure_client(api_id).await?;

        let me = client
            .get_me()
            .await
            .map_err(|e| AppError::Telegram(format!("get_me failed: {e}")))?;

        let avatar_path_opt = if let Some(photo) = Peer::User(me.clone()).photo(false).await {
            let avatar_dir = self.store_dir.join("profile");
            fs::create_dir_all(&avatar_dir).await?;
            let avatar_path = avatar_dir.join(format!("avatar-{}.jpg", me.id()));
            client
                .download_media(&photo, &avatar_path)
                .await
                .map_err(|e| AppError::Telegram(format!("download profile photo failed: {e}")))?;
            Some(avatar_path.to_string_lossy().to_string())
        } else {
            None
        };

        Ok(UserProfileDto {
            display_name: {
                let name = me.full_name();
                if name.trim().is_empty() {
                    me.username().unwrap_or("Telegram").to_string()
                } else {
                    name
                }
            },
            username: me.username().map(ToOwned::to_owned),
            phone_masked: me.phone().map(mask_phone),
            avatar_path_opt,
        })
    }

    pub async fn logout(&self) -> AppResult<()> {
        if self.mock_mode {
            let mut auth = self.auth_lock()?;
            auth.state = AuthState::LoggedOut;
            auth.client = None;
            auth.runner_task = None;
            auth.login_token = None;
            auth.password_token = None;
            self.upload_pool.shutdown();
            return Ok(());
        }

        let maybe_client = self.auth_lock()?.client.clone();
        if let Some(client) = maybe_client {
            let _ = client.sign_out().await;
        }

        {
            let mut auth = self.auth_lock()?;
            if let Some(task) = auth.runner_task.take() {
                task.abort();
            }
            auth.state = AuthState::LoggedOut;
            auth.client = None;
            auth.login_token = None;
            auth.password_token = None;
        }

        self.upload_pool.shutdown();
        let _ = fs::remove_file(&self.session_path).await;
        Ok(())
    }

    // ── Upload ────────────────────────────────────────────────────────────────

    /// Upload de chunk usando o pool (round-robin entre UPLOAD_POOL_SIZE conexões).
    pub async fn upload_chunk(&self, payload: Vec<u8>, original_name: &str) -> AppResult<String> {
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

        // Tenta o pool de upload primeiro; se não estiver pronto cai no cliente principal
        let client = self
            .upload_pool
            .next()
            .or_else(|_| {
                let api_id = self
                    .auth_lock()?
                    .api_id
                    .ok_or_else(|| AppError::Telegram("missing api_id".to_string()))?;
                // fallback síncrono não funciona aqui — propaga o erro do pool
                Err(AppError::Telegram(format!(
                    "upload pool indisponível; api_id={api_id}"
                )))
            })
            .or_else(|_| {
                // último recurso: cliente de auth
                self.auth_lock()?
                    .client
                    .clone()
                    .ok_or_else(|| AppError::Telegram("no client available".to_string()))
            })?;

        let peer = self.saved_messages_peer(&client).await?;
        let file_name = sanitize_upload_name(original_name);

        self.with_retry(|| {
            let client = client.clone();
            let peer = peer;
            let payload = payload.clone();
            let file_name = file_name.clone();
            async move {
                let size = payload.len();
                let mut stream = Cursor::new(payload);
                let uploaded = client
                    .upload_stream(&mut stream, size, file_name)
                    .await
                    .map_err(|e| {
                        AppError::Telegram(format!("telegram upload_stream failed: {e}"))
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
        .await
    }

    pub async fn upload_file_path(&self, source_path: &Path) -> AppResult<String> {
        self.ensure_auth()?;
        if self.mock_mode {
            return self
                .with_retry(|| {
                    let source_path = source_path.to_path_buf();
                    async move {
                        let id = Uuid::new_v4().to_string();
                        let file_name = source_path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("file.bin");
                        let path = self.store_dir.join(format!("{id}-{file_name}"));
                        fs::copy(&source_path, path).await?;
                        Ok(id)
                    }
                })
                .await;
        }

        let api_id = self
            .auth_lock()?
            .api_id
            .ok_or_else(|| AppError::Telegram("missing api_id in auth context".to_string()))?;
        let client = self.ensure_client(api_id).await?;
        let peer = self.saved_messages_peer(&client).await?;
        let source_path = source_path.to_path_buf();

        self.with_retry(|| {
            let client = client.clone();
            let source_path = source_path.clone();
            async move {
                let uploaded = client
                    .upload_file(&source_path)
                    .await
                    .map_err(|e| AppError::Telegram(format!("upload_file failed: {e}")))?;
                let message = client
                    .send_message(
                        peer,
                        InputMessage::new().text("tgdrive-file").file(uploaded),
                    )
                    .await
                    .map_err(|e| AppError::Telegram(format!("send_message failed: {e}")))?;
                Ok(message.id().to_string())
            }
        })
        .await
    }

    // ── Download ──────────────────────────────────────────────────────────────

    pub async fn download_to_path(&self, message_id: &str, dest: &Path) -> AppResult<()> {
        self.ensure_auth()?;
        if self.mock_mode {
            let id = message_id.to_string();
            let src = self.store_dir.join(&id);
            if src.exists() {
                fs::copy(&src, dest).await?;
            } else {
                fs::write(dest, b"mock-chunk-data").await?;
            }
            return Ok(());
        }

        let api_id = self
            .auth_lock()?
            .api_id
            .ok_or_else(|| AppError::Telegram("missing api_id".to_string()))?;
        let client = self.ensure_client(api_id).await?;
        let peer = self.saved_messages_peer(&client).await?;
        let msg_id: i32 = message_id
            .parse()
            .map_err(|_| AppError::Validation(format!("invalid message id: {message_id}")))?;

        self.with_retry(|| {
            let client = client.clone();
            let dest = dest.to_path_buf();
            async move {
                let messages = client
                    .get_messages_by_id(peer, &[msg_id])
                    .await
                    .map_err(|e| AppError::Telegram(format!("get_messages_by_id failed: {e}")))?;
                let msg = messages
                    .into_iter()
                    .next()
                    .flatten()
                    .ok_or_else(|| AppError::Telegram(format!("message {msg_id} not found")))?;

                // download_media aceita qualquer tipo Downloadable.
                // msg.media() retorna Option<Media> — passamos por referência.
                // Sem import explícito do enum para evitar problemas de caminho
                // entre versões do grammers.
                match msg.media() {
                    Some(media) => {
                        client.download_media(&media, &dest).await.map_err(|e| {
                            AppError::Telegram(format!("download_media failed: {e}"))
                        })?;
                    }
                    None => {
                        return Err(AppError::Telegram(format!(
                            "message {msg_id} has no downloadable media"
                        )));
                    }
                }

                Ok(())
            }
        })
        .await
    }

    pub async fn download_chunk_bytes(&self, message_id: &str) -> AppResult<Vec<u8>> {
        self.ensure_auth()?;
        if self.mock_mode {
            let src = self.store_dir.join(message_id);
            if src.exists() {
                return Ok(fs::read(&src).await?);
            }
            return Ok(vec![0u8; 1024]);
        }

        let tmp = self.store_dir.join(format!("dl-{}.tmp", Uuid::new_v4()));
        self.download_to_path(message_id, &tmp).await?;
        let bytes = fs::read(&tmp).await?;
        let _ = fs::remove_file(&tmp).await;
        Ok(bytes)
    }

    // ── Aliases para compatibilidade com auth.rs e downloader.rs ─────────────

    /// Alias usado por auth.rs — tenta restaurar sessão já existente em disco.
    pub async fn restore_runtime_auth(&self) -> AppResult<AuthState> {
        self.restore_session().await
    }

    /// Alias usado por downloader.rs para arquivos únicos (não-chunked).
    pub async fn download_file_to_path(&self, message_id: &str, dest: &Path) -> AppResult<()> {
        self.download_to_path(message_id, dest).await
    }

    /// Alias usado por downloader.rs para chunks individuais.
    pub async fn download_chunk(&self, message_id: &str) -> AppResult<Vec<u8>> {
        self.download_chunk_bytes(message_id).await
    }

    // ── Session blob (para auth.rs) ───────────────────────────────────────────

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

    // ── Helpers privados ──────────────────────────────────────────────────────

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

    async fn with_retry<F, Fut, T>(&self, mut f: F) -> AppResult<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = AppResult<T>>,
    {
        const MAX: u32 = 5;
        let mut delay = Duration::from_millis(500);
        for attempt in 0..MAX {
            match f().await {
                Ok(v) => return Ok(v),
                Err(e) if attempt + 1 < MAX => {
                    warn!(attempt, error = %e, "telegram op failed, retrying");
                    sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(8));
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }
}

// ── Utilitários ───────────────────────────────────────────────────────────────

fn sanitize_upload_name(name: &str) -> String {
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");
    let random: u32 = rand::thread_rng().gen();
    format!("chunk-{random:08x}.{ext}")
}

fn mask_phone(phone: &str) -> String {
    if phone.len() <= 4 {
        return "***".to_string();
    }
    let visible = &phone[phone.len() - 4..];
    format!("***{visible}")
}
