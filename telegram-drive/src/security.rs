use crate::models::{AppError, AppResult};
use pbkdf2::pbkdf2_hmac_array;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::{Mutex, OnceLock};

const KEY_BYTES: usize = 32;
const SALT_BYTES: usize = 32;
const PBKDF2_ITERATIONS: u32 = 600_000;
#[cfg(not(test))]
const KEYRING_SERVICE: &str = "com.savedrive.desktop";

pub fn derive_local_key(salt_path: &Path, purpose: &str) -> AppResult<[u8; KEY_BYTES]> {
    let salt = load_or_create_salt(salt_path)?;
    let base_secret = load_or_create_platform_secret(secret_label(salt_path, purpose))?;
    derive_key_from_material(&base_secret, &salt, purpose)
}

pub fn derive_legacy_local_key(salt_path: &Path, purpose: &str) -> AppResult<[u8; KEY_BYTES]> {
    let salt = load_or_create_salt(salt_path)?;
    derive_key_from_material(&local_seed_material(), &salt, purpose)
}

fn derive_key_from_material(
    material: &str,
    salt: &[u8; SALT_BYTES],
    purpose: &str,
) -> AppResult<[u8; KEY_BYTES]> {
    if material.is_empty() {
        return Err(AppError::Crypto(
            "platform secret material is empty".to_string(),
        ));
    }

    let mut purpose_bound_salt = Vec::with_capacity(salt.len() + purpose.len());
    purpose_bound_salt.extend_from_slice(salt);
    purpose_bound_salt.extend_from_slice(purpose.as_bytes());
    Ok(pbkdf2_hmac_array::<Sha256, KEY_BYTES>(
        material.as_bytes(),
        &purpose_bound_salt,
        PBKDF2_ITERATIONS,
    ))
}

fn load_or_create_salt(path: &Path) -> AppResult<[u8; SALT_BYTES]> {
    if let Ok(bytes) = std::fs::read(path) {
        if bytes.len() == SALT_BYTES {
            let mut salt = [0u8; SALT_BYTES];
            salt.copy_from_slice(&bytes);
            return Ok(salt);
        }
        backup_corrupt_salt(path)?;
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut salt = [0u8; SALT_BYTES];
    OsRng.fill_bytes(&mut salt);
    let tmp = tmp_path(path);
    std::fs::write(&tmp, salt)?;
    std::fs::rename(&tmp, path)?;
    Ok(salt)
}

fn backup_corrupt_salt(path: &Path) -> AppResult<()> {
    if !path.exists() {
        return Ok(());
    }
    let backup = path.with_extension(format!(
        "corrupt-{}.salt",
        chrono::Utc::now().timestamp()
    ));
    std::fs::rename(path, backup)?;
    Ok(())
}

fn tmp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("salt.bin");
    path.with_file_name(format!("{name}.tmp"))
}

fn secret_label(salt_path: &Path, purpose: &str) -> String {
    let digest = Sha256::digest(salt_path.to_string_lossy().as_bytes());
    format!("{}:{}", purpose, hex::encode(digest))
}

#[cfg(not(test))]
fn load_or_create_platform_secret(label: String) -> AppResult<String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, &label)
        .map_err(|e| AppError::Crypto(format!("keyring entry init failed: {e}")))?;
    match entry.get_password() {
        Ok(secret) if !secret.is_empty() => Ok(secret),
        Ok(_) => Err(AppError::Crypto(
            "platform keyring returned empty secret".to_string(),
        )),
        Err(keyring::Error::NoEntry) => {
            let secret = generate_secret();
            entry
                .set_password(&secret)
                .map_err(|e| AppError::Crypto(format!("keyring set_password failed: {e}")))?;
            Ok(secret)
        }
        Err(e) => Err(AppError::Crypto(format!(
            "keyring get_password failed: {e}"
        ))),
    }
}

#[cfg(test)]
fn load_or_create_platform_secret(label: String) -> AppResult<String> {
    static STORE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let store = STORE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = store
        .lock()
        .map_err(|_| AppError::Concurrency("test secret store poisoned".to_string()))?;
    let value = guard
        .entry(label)
        .or_insert_with(generate_secret)
        .clone();
    Ok(value)
}

fn generate_secret() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn local_seed_material() -> String {
    let exe = std::env::current_exe()
        .ok()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "unknown-exe".to_string());
    let home = dirs::home_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "unknown-home".to_string());
    format!(
        "{}:{}:{}:{}:{}",
        std::env::consts::OS,
        std::env::var("USERNAME").unwrap_or_else(|_| "user".to_string()),
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "host".to_string()),
        home,
        exe
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_stable_key_with_platform_secret_and_persisted_salt() {
        let temp = tempfile::tempdir().unwrap();
        let salt_path = temp.path().join("session.salt");
        let a = derive_local_key(&salt_path, "test-purpose").unwrap();
        let b = derive_local_key(&salt_path, "test-purpose").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn legacy_and_current_key_differ() {
        let temp = tempfile::tempdir().unwrap();
        let salt_path = temp.path().join("session.salt");
        let current = derive_local_key(&salt_path, "test-purpose").unwrap();
        let legacy = derive_legacy_local_key(&salt_path, "test-purpose").unwrap();
        assert_ne!(current, legacy);
    }

    #[test]
    fn recovers_from_corrupt_salt() {
        let temp = tempfile::tempdir().unwrap();
        let salt_path = temp.path().join("session.salt");
        std::fs::write(&salt_path, b"short").unwrap();
        let key = derive_local_key(&salt_path, "test-purpose").unwrap();
        assert_eq!(key.len(), KEY_BYTES);
        let backups = std::fs::read_dir(temp.path())
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("corrupt-")
            })
            .count();
        assert_eq!(backups, 1);
    }
}
