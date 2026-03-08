use crate::models::AppResult;
use pbkdf2::pbkdf2_hmac_array;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::Sha256;
use std::path::{Path, PathBuf};

const KEY_BYTES: usize = 32;
const SALT_BYTES: usize = 32;
const PBKDF2_ITERATIONS: u32 = 600_000;

pub fn derive_local_key(salt_path: &Path, purpose: &str) -> AppResult<[u8; KEY_BYTES]> {
    let salt = load_or_create_salt(salt_path)?;
    let seed = local_seed_material();
    let mut purpose_bound_salt = Vec::with_capacity(salt.len() + purpose.len());
    purpose_bound_salt.extend_from_slice(&salt);
    purpose_bound_salt.extend_from_slice(purpose.as_bytes());
    Ok(pbkdf2_hmac_array::<Sha256, KEY_BYTES>(
        seed.as_bytes(),
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
    fn derives_stable_key_with_persisted_salt() {
        let temp = tempfile::tempdir().unwrap();
        let salt_path = temp.path().join("session.salt");
        let a = derive_local_key(&salt_path, "test-purpose").unwrap();
        let b = derive_local_key(&salt_path, "test-purpose").unwrap();
        assert_eq!(a, b);
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
