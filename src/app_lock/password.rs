//! A local UI gate, not encryption of the host database. Only a salted
//! Argon2id verifier is stored; passwords never leave the worker's memory.
use std::{fs, io, io::Write, path::Path};

use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq as _;
use zeroize::Zeroizing;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Verifier {
    version: u8,
    salt: [u8; 16],
    hash: [u8; 32],
}

impl Verifier {
    pub(super) fn new(password: &str) -> anyhow::Result<Self> {
        let mut salt = [0; 16];
        rand::rng().fill_bytes(&mut salt);
        Ok(Self {
            version: 1,
            salt,
            hash: *derive(password, &salt)?,
        })
    }

    pub(super) fn verify(&self, password: &str) -> anyhow::Result<bool> {
        anyhow::ensure!(self.version == 1, "unsupported app lock version");
        Ok(bool::from(
            self.hash.ct_eq(derive(password, &self.salt)?.as_ref()),
        ))
    }
}

fn derive(password: &str, salt: &[u8; 16]) -> anyhow::Result<Zeroizing<[u8; 32]>> {
    let params = Params::new(64 * 1024, 3, 1, Some(32)).map_err(|e| anyhow::anyhow!(e))?;
    let mut hash = Zeroizing::new([0; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, hash.as_mut())
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(hash)
}

/// Missing means disabled. Unreadable, truncated or unknown files fail closed.
pub(super) fn load(path: &Path) -> anyhow::Result<Option<Verifier>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let verifier: Verifier = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(verifier.version == 1, "unsupported app lock version");
    Ok(Some(verifier))
}

/// Security settings commit to disk before the UI reports success.
pub(super) fn save(path: &Path, verifier: Option<&Verifier>) -> anyhow::Result<()> {
    let Some(verifier) = verifier else {
        fs::remove_file(path)?;
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> anyhow::Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&serde_json::to_vec(verifier)?)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{Verifier, load, save};
    use std::fs;

    #[test]
    fn passwords_are_salted_and_round_trip_without_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app-lock.json");
        assert!(load(&path).unwrap().is_none());
        let first = Verifier::new("a long 密码 123").unwrap();
        let second = Verifier::new("a long 密码 123").unwrap();
        assert_ne!(first.salt, second.salt);
        assert_ne!(first.hash, second.hash);
        save(&path, Some(&first)).unwrap();
        assert!(
            !fs::read_to_string(&path)
                .unwrap()
                .contains("a long 密码 123")
        );
        let loaded = load(&path).unwrap().unwrap();
        assert!(loaded.verify("a long 密码 123").unwrap());
        assert!(!loaded.verify("wrong password").unwrap());
        assert!(!loaded.verify("a long 密码 123 ").unwrap());
        save(&path, None).unwrap();
        assert!(load(&path).unwrap().is_none());
    }

    #[test]
    fn corrupt_and_future_files_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app-lock.json");
        fs::write(&path, b"{").unwrap();
        assert!(load(&path).is_err());
        let mut verifier = Verifier::new("password123").unwrap();
        verifier.version = 2;
        save(&path, Some(&verifier)).unwrap();
        assert!(load(&path).is_err());
    }
}
