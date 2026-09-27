use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use tempfile::Builder;
use thiserror::Error;

use crate::crypto::{self, CryptoError};
use crate::format::KdfParams;
use crate::notes::{CONTENT_VERSION, NoteError, Notebook};

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("{0}")]
    Locked(String),
    #[error("a vault already exists at {0}")]
    Exists(PathBuf),
    #[error("no vault found at {0} (run `coffer init` first)")]
    NotFound(PathBuf),
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error("vault contents are invalid: {0}")]
    Corrupt(String),
    #[error("failed to serialize vault contents: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error(transparent)]
    Note(#[from] NoteError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub struct Vault {
    path: PathBuf,
    notebook: Notebook,
    params: KdfParams,
    _lock: File,
}

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vault")
            .field("path", &self.path)
            .field("notes", &self.notebook.notes.len())
            .finish_non_exhaustive()
    }
}

/// Create the vault directory with owner-only permissions if it did not
/// already exist.
fn ensure_dir(dir: &Path) -> Result<(), VaultError> {
    if dir.as_os_str().is_empty() {
        return Ok(());
    }
    let existed = dir.exists();
    fs::create_dir_all(dir)?;
    if !existed {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

/// Read the pid the lock holder wrote into `vault.lock`, if any.
fn lock_holder_pid(lock: &File) -> Option<u32> {
    let mut file = lock;
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).ok()?;
    contents.trim().parse().ok()
}

fn acquire_lock(vault_path: &Path) -> Result<File, VaultError> {
    let dir = match vault_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    ensure_dir(&dir)?;
    let lock_path = dir.join("vault.lock");
    let mut lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(lock_path)?;
    match lock.try_lock() {
        Ok(()) => {
            // Record our pid for diagnostics; only we can hold the lock now.
            let _ = lock.set_len(0);
            let _ = lock.seek(SeekFrom::Start(0));
            let _ = write!(lock, "{}", std::process::id());
            let _ = lock.flush();
            Ok(lock)
        }
        Err(TryLockError::WouldBlock) => {
            let message = match lock_holder_pid(&lock) {
                Some(pid) => format!(
                    "another coffer process (pid {pid}) is using this vault; \
                     kill {pid} if that process is stale"
                ),
                None => "another coffer process is using this vault".to_string(),
            };
            Err(VaultError::Locked(message))
        }
        Err(TryLockError::Error(e)) => Err(VaultError::Io(e)),
    }
}

/// Write `bytes` to `path` atomically: temp file in the same directory,
/// fsync, rename over the target, fsync the directory.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), VaultError> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let mut tmp = Builder::new().prefix(".vault-tmp-").tempfile_in(&dir)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| VaultError::Io(e.error))?;
    File::open(&dir)?.sync_all()?;
    Ok(())
}

impl Vault {
    /// Create a new vault. Fails if the file already exists with content.
    pub fn init(path: impl Into<PathBuf>, password: &[u8]) -> Result<Self, VaultError> {
        Self::init_with_params(path, password, KdfParams::default())
    }

    /// Like [`Vault::init`], but with explicit key derivation parameters.
    pub fn init_with_params(
        path: impl Into<PathBuf>,
        password: &[u8],
        params: KdfParams,
    ) -> Result<Self, VaultError> {
        let path = path.into();
        if let Ok(meta) = fs::metadata(&path)
            && meta.len() > 0
        {
            return Err(VaultError::Exists(path));
        }
        let lock = acquire_lock(&path)?;
        let vault = Self {
            path,
            notebook: Notebook::new(),
            params,
            _lock: lock,
        };
        vault.save(password)?;
        Ok(vault)
    }

    /// Unlock an existing vault and load its contents.
    pub fn open(path: impl Into<PathBuf>, password: &[u8]) -> Result<Self, VaultError> {
        let path = path.into();
        let lock = acquire_lock(&path)?;
        let data = match fs::read(&path) {
            Ok(data) => data,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(VaultError::NotFound(path));
            }
            Err(e) => return Err(VaultError::Io(e)),
        };
        let opened = crypto::open(password, &data)?;
        let notebook: Notebook = serde_json::from_slice(&opened.plaintext)
            .map_err(|e| VaultError::Corrupt(e.to_string()))?;
        if notebook.version != CONTENT_VERSION {
            return Err(VaultError::Corrupt(format!(
                "unsupported content version {}",
                notebook.version
            )));
        }
        Ok(Self {
            path,
            notebook,
            params: opened.params,
            _lock: lock,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn notebook(&self) -> &Notebook {
        &self.notebook
    }

    pub fn notebook_mut(&mut self) -> &mut Notebook {
        &mut self.notebook
    }

    /// Encrypt the current contents and write them out atomically, always
    /// with a fresh salt and nonce and the vault's KDF parameters.
    pub fn save(&self, password: &[u8]) -> Result<(), VaultError> {
        let plaintext = zeroize::Zeroizing::new(serde_json::to_vec(&self.notebook)?);
        let sealed = crypto::seal_with_params(password, &plaintext, self.params)?;
        atomic_write(&self.path, &sealed)
    }

    /// Re-encrypt the vault under a new password.
    pub fn rekey(&self, new_password: &[u8]) -> Result<(), VaultError> {
        self.save(new_password)
    }
}
