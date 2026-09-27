# coffer

A small, text-only note-taking CLI. Every note lives in a single encrypted
vault file. No formatting, no sync daemon, no network code — just notes that
are unreadable without your password.

```
$ coffer new -t "wifi"
$ coffer list
   1  2026-09-27T09:14:03  wifi
   2  2026-09-27T09:15:41  shopping list
$ coffer show wifi
SSID: home
password: hunter2
```

## Build

```
cargo build --release
# binary at target/release/coffer
```

Requires stable Rust. No other system dependencies.

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

Notes are addressed by numeric id or by exact (case-insensitive) title; an
ambiguous title is rejected with a request to use the id.

### Vault location

Defaults to `~/coffer-vault/vault`. Override per-invocation or globally:

```
coffer --vault /path/to/vault list
export COFFER_VAULT=/path/to/vault
```

### Password input

- Interactively, the password is read from `/dev/tty` with echo disabled.
- For scripts, `--password-stdin` reads the password from the **first line**
  of stdin. For `coffer new`, the rest of stdin is the note body:

```
printf 'my passphrase\nremember the milk\n' | coffer --password-stdin new
```

`coffer passwd` always requires a terminal, because it must ask for a new
password without it appearing in shell history or process arguments.

### Editor

`coffer new` and `coffer edit` open `$VISUAL`, then `$EDITOR`, then the
first of `nano`, `vim` or `vi` found on your `PATH` (with a short nano
key-binding reminder when nano is auto-selected). The edited text is written
to a temporary file with owner-only permissions, preferring
`$XDG_RUNTIME_DIR` (a tmpfs on Linux), and the file is deleted as soon as the
editor exits.

To always use a particular editor, set it in your shell profile:

```
export EDITOR=nano
```

**This is the one moment decrypted text is briefly on disk**, so a plaintext
cloud-synced or non-tmpfs temp directory weakens the model. If that matters
to you, pipe the body in via stdin instead of using the editor.

### If the vault is locked

coffer holds an exclusive lock so two runs cannot overwrite each other. If a
connection dies while the editor is open, the old process can survive the
dropped session and keep holding the lock. coffer tells you who holds it:

```
$ coffer list
error: another coffer process (pid 12345) is using this vault; kill 12345 if that process is stale
```

Kill that pid and retry. The lock is released automatically when the process
exits, and no data can be lost this way because vault writes are atomic.
Leftover plaintext editor temp files from dead processes are cleaned up
automatically the next time an editor is opened.

## Security design

| Layer | Choice |
| --- | --- |
| Key derivation | Argon2id v1.3, 64 MiB memory, 3 iterations, 1 lane, 32-byte random salt |
| Encryption | XChaCha20-Poly1305 (RFC 8439 plus the 192-bit-nonce XChaCha construction) |
| Nonces | 24 random bytes, drawn from the OS CSPRNG for every write |
| Integrity | AEAD tag over ciphertext **and** the full header (magic, KDF params, salt, nonce) |
| Disk writes | temp file in the same directory, `fsync`, atomic `rename`, directory `fsync` |
| Concurrency | exclusive `flock` on `vault.lock`; a second process refuses to run |
| Permissions | vault directory `0700`, vault and lock files `0600` |
| Memory | derived keys and decrypted buffers are zeroized on drop |
| Dependencies | small, mainstream RustCrypto crates; `cargo audit` runs in CI |

Wrong password and corrupted vault produce the same generic error, so the
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

The header is passed to the AEAD as associated data, so changing any byte of
it (including the KDF parameters) makes decryption fail. KDF parameters are
read from the header, which allows raising the defaults in future versions
without breaking existing vaults. The decrypted payload is a versioned JSON
document containing ids, titles, bodies and timestamps.

### Threat model

**Protected against:** someone who obtains the vault file — a stolen disk,
a backup, another user on the machine, or the contents of a private GitHub
repo — learning anything about your notes without the password. Metadata is
encrypted too: titles, timestamps and note count are inside the ciphertext.
The only thing observable is the approximate vault size.

**Not protected against:**

- A compromised machine: keyloggers, memory dumps while coffer is running,
  malicious software running as your user.
- Swap or hibernation writing decrypted memory to disk. Use encrypted swap.
- Weak passwords. Offline guessing is deliberately expensive (64 MiB of
  Argon2id per attempt) but not impossible; use a long passphrase.
- Rollback: an attacker who can replace the vault with an older copy can
  make you see — and overwrite — stale data.
- The editor temporary file described above.

There is no recovery mechanism. **If you lose the password, the notes are
gone.**

## Backing up with a private GitHub repo

The vault is already encrypted, so a private remote is a fine backup target.

```
cd ~/coffer-vault
git init -b main
printf 'vault.lock\n.vault-tmp-*\n' > .gitignore
git add .gitignore vault
git commit -m "vault backup"
git remote add origin git@github.com:<you>/coffer-vault.git
git push -u origin main
```

Committing again after changes snapshots the ciphertext; atomic writes mean
you can commit at any time without catching a half-written file. Remember
that old commits keep old ciphertext under old passwords after `passwd` —
delete the repo history too if you rotate a leaked password.

## License

DO WHATEVER YOU WANT LICENSE (see `LICENSE`).
