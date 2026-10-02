# coffer

coffer is a text-only note-taking CLI. All notes live in a single encrypted
vault file that is unreadable without the password, and there is no formatting,
no sync daemon, and no network code.

The output below is from a real session against a throwaway vault.

```
$ printf 'correct horse battery staple\nSSID: home\npassword: hunter2\n' | coffer --password-stdin new -t wifi
added note 1
$ printf 'correct horse battery staple\nrenew the domain\n' | coffer --password-stdin new
added note 2
$ printf 'correct horse battery staple\n' | coffer --password-stdin list
   1  2026-10-02T12:10:32  wifi
   2  2026-10-02T12:10:32  renew the domain
$ printf 'correct horse battery staple\n' | coffer --password-stdin show wifi
SSID: home
password: hunter2
```

## Build

Requires stable Rust and no other system dependencies.

```
cargo build --release
# binary at target/release/coffer
```

The CI workflow runs `cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings`, and `cargo test`, then audits dependencies with
`rustsec/audit-check`.

## Usage

| Command | Description |
| --- | --- |
| `coffer init` | Create a new vault (asks for the password twice) |
| `coffer new [-t TITLE]` | Add a note; opens `$EDITOR`, or reads piped stdin |
| `coffer list` | List note ids, timestamps and titles |
| `coffer show <id\|title>` | Print a note's body |
| `coffer edit <id\|title>` | Edit a note in `$EDITOR` |
| `coffer rm [-f] <id\|title>` | Delete a note |
| `coffer search <term>` | Case-insensitive search of titles and bodies |
| `coffer passwd` | Change the master password |

Notes are addressed by numeric id or by exact (case-insensitive) title. An
ambiguous title is rejected with a request to use the id. A note created without
`-t` takes its title from the first non-empty line of the body, truncated to 80
characters.

`coffer rm` asks for confirmation on a terminal and refuses to delete anything
on a non-interactive stdin unless `--force` is passed.

### Vault location

The vault defaults to `~/coffer-vault/vault`. Override it per-invocation or
globally:

```
coffer --vault /path/to/vault list
export COFFER_VAULT=/path/to/vault
```

### Password input

Interactively, the password is read from `/dev/tty` with echo disabled. For
scripts, `--password-stdin` reads the password from the first line of stdin. For
`coffer new`, the rest of stdin is the note body:

```
printf 'my passphrase\nremember the milk\n' | coffer --password-stdin new
```

Passwords shorter than 12 characters produce a warning. `coffer passwd` needs an
interactive terminal to read the new password, so that it never appears in shell
history or process arguments.

### Editor

`coffer new` and `coffer edit` open `$VISUAL`, then `$EDITOR`, then the first of
`nano`, `vim`, or `vi` found on `PATH`. When nano is auto-selected, coffer prints
a short key-binding reminder. The edited text is written to a temporary file with
owner-only permissions, preferring `$XDG_RUNTIME_DIR` (a tmpfs on Linux), and the
file is deleted as soon as the editor exits.

To always use a particular editor, set it in the shell profile:

```
export EDITOR=nano
```

This is the one point where decrypted text is briefly on disk. A plaintext,
non-tmpfs temporary directory weakens the model, so pipe the body in via stdin
instead of using the editor if that matters.

### Locking

coffer takes an exclusive `flock` on `vault.lock` so two runs cannot overwrite
each other. If a process survives a dropped session and keeps holding the lock,
coffer reports the holder's pid:

```
$ coffer list
error: another coffer process (pid 12345) is using this vault; kill 12345 if that process is stale
```

The block above shows the error format; the pid is an example.

Kill the pid and retry. The lock is released when the process exits, and vault
writes are atomic, so no data is lost this way. Leftover plaintext editor
temporary files from dead processes are removed the next time an editor is
opened.

## Security

| Layer | Choice |
| --- | --- |
| Key derivation | Argon2id v1.3, 64 MiB memory, 3 iterations, 1 lane, 32-byte random salt |
| Encryption | XChaCha20-Poly1305 |
| Nonces | 24 random bytes from the OS CSPRNG on every write |
| Integrity | AEAD tag over the ciphertext and the full header (magic, KDF params, salt, nonce) |
| Disk writes | temp file in the same directory, `fsync`, atomic `rename`, directory `fsync` |
| Concurrency | exclusive `flock` on `vault.lock`; a second process refuses to run |
| Permissions | vault directory `0700`, vault and lock files `0600` |
| Memory | derived keys and decrypted buffers are zeroized on drop |
| Dependencies | RustCrypto crates for Argon2id and XChaCha20-Poly1305; CI audits dependencies with `rustsec/audit-check` |

A wrong password and a corrupted vault produce the same generic error, so the
tool is not an oracle for guessing.

### Vault format (version 1)

```
offset  size  field
0       8     magic "COFFERv1"
8       1     KDF id (1 = Argon2id)
9       4     Argon2 memory cost, KiB (little-endian)
13      4     Argon2 time cost
17      4     Argon2 parallelism
21      32    salt
53      24    nonce
77      ...   XChaCha20-Poly1305 ciphertext || 16-byte tag
```

The header is passed to the AEAD as associated data, so changing any byte of it,
including the KDF parameters, makes decryption fail. KDF parameters are read from
the header, which allows raising the defaults in a future version without
breaking existing vaults. The decrypted payload is a versioned JSON document
containing ids, titles, bodies, and timestamps.

### Threat model

Protected against someone who obtains the vault file — a stolen disk, a backup,
another user on the machine, or the contents of a private GitHub repo — learning
anything about the notes without the password. Metadata is encrypted too: titles,
timestamps, and note count are inside the ciphertext. The only observable
property is the approximate vault size.

Not protected against:

- A compromised machine: keyloggers, memory dumps while coffer is running, or
  malicious software running as the user.
- Swap or hibernation writing decrypted memory to disk. Use encrypted swap.
- Weak passwords. Offline guessing costs 64 MiB of Argon2id per attempt but is
  not impossible; use a long passphrase.
- Rollback: an attacker who can replace the vault with an older copy can make
  the user see and overwrite stale data.
- The editor temporary file described above.

There is no recovery mechanism. If the password is lost, the notes are gone.

## Backing up with a private GitHub repo

The vault is encrypted, so a private remote is a valid backup target.

```
cd ~/coffer-vault
git init -b main
printf 'vault.lock\n.vault-tmp-*\n' > .gitignore
git add .gitignore vault
git commit -m "vault backup"
git remote add origin git@github.com:<you>/coffer-vault.git
git push -u origin main
```

Committing again after changes snapshots the ciphertext; atomic writes mean the
vault can be committed at any time without catching a half-written file. Old
commits keep old ciphertext under old passwords after `passwd`, so delete the
repo history as well if a leaked password is rotated.

## License

DO WHATEVER YOU WANT LICENSE (see `LICENSE`).
