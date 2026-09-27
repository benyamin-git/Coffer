use std::fs;

use coffer::format::KdfParams;
use coffer::vault::{Vault, VaultError};

const PASSWORD: &[u8] = b"correct horse battery staple";

// Weak parameters keep the tests fast; production always uses the defaults.
const FAST: KdfParams = KdfParams {
    m_cost: 8,
    t_cost: 1,
    p_cost: 1,
};

fn vault_path(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("vault")
}

fn init(path: &std::path::Path) -> Vault {
    Vault::init_with_params(path, PASSWORD, FAST).expect("init")
}

#[test]
fn init_save_reopen_roundtrip() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let path = vault_path(&dir);

    let mut vault = init(&path);
    let id = vault.notebook_mut().add("first", "hello vault");
    vault.notebook_mut().add("second", "line one\nline two");
    vault.save(PASSWORD).expect("save");
    drop(vault);

    let vault = Vault::open(&path, PASSWORD).expect("open");
    assert_eq!(vault.notebook().notes.len(), 2);
    let note = vault.notebook().get(id).expect("note");
    assert_eq!(note.title, "first");
    assert_eq!(note.body, "hello vault");
}

#[test]
fn wrong_password_cannot_open() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let path = vault_path(&dir);
    let mut vault = init(&path);
    vault.notebook_mut().add("t", "b");
    vault.save(PASSWORD).expect("save");
    drop(vault);

    let err = Vault::open(&path, b"not the password").unwrap_err();
    assert!(matches!(err, VaultError::Crypto(_)));
}

#[test]
fn rekey_invalidates_old_password() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let path = vault_path(&dir);
    let mut vault = init(&path);
    vault.notebook_mut().add("t", "body");
    vault.save(PASSWORD).expect("save");
    vault.rekey(b"a brand new passphrase").expect("rekey");
    drop(vault);

    assert!(Vault::open(&path, PASSWORD).is_err());
    let vault = Vault::open(&path, b"a brand new passphrase").expect("open new");
    assert_eq!(vault.notebook().notes.len(), 1);
}

#[test]
fn refuses_to_overwrite_existing_vault() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let path = vault_path(&dir);
    let vault = init(&path);
    drop(vault);
    let err = Vault::init(&path, PASSWORD).unwrap_err();
    assert!(matches!(err, VaultError::Exists(_)));
}

#[test]
fn missing_vault_is_reported_as_not_found() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let path = vault_path(&dir);
    let err = Vault::open(&path, PASSWORD).unwrap_err();
    assert!(matches!(err, VaultError::NotFound(_)));
}

#[test]
fn second_process_cannot_lock_the_vault() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let path = vault_path(&dir);
    let first = init(&path);
    match Vault::open(&path, PASSWORD).unwrap_err() {
        VaultError::Locked(message) => assert!(
            message.contains(&std::process::id().to_string()),
            "lock message should name the holding pid: {message}"
        ),
        other => panic!("expected Locked, got {other:?}"),
    }
    drop(first);
    Vault::open(&path, PASSWORD).expect("lock released after drop");
}

#[test]
fn atomic_writes_leave_no_temp_files() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let path = vault_path(&dir);
    let mut vault = init(&path);
    vault.notebook_mut().add("t", "b");
    vault.save(PASSWORD).expect("save");

    let leftovers: Vec<String> = fs::read_dir(dir.path())
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".vault-tmp-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temp files left behind: {leftovers:?}"
    );
}

#[test]
fn fresh_salt_and_nonce_on_every_save() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let path = vault_path(&dir);
    let mut vault = init(&path);
    vault.notebook_mut().add("t", "b");

    vault.save(PASSWORD).expect("save 1");
    let first = fs::read(&path).expect("read");
    vault.save(PASSWORD).expect("save 2");
    let second = fs::read(&path).expect("read");

    assert_ne!(first, second);
}

#[cfg(unix)]
#[test]
fn vault_files_have_owner_only_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tmpdir");
    let path = vault_path(&dir);
    let vault = init(&path);
    drop(vault);

    let file_mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
    assert_eq!(file_mode, 0o600);
}

#[test]
fn corrupt_vault_file_is_rejected() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let path = vault_path(&dir);
    fs::write(&path, b"definitely not a vault").expect("write");
    let err = Vault::open(&path, PASSWORD).unwrap_err();
    assert!(matches!(err, VaultError::Crypto(_)));
}
