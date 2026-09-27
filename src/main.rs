mod cli;

use std::io::{self, IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::Parser;
use zeroize::Zeroizing;

use cli::{Cli, Command};
use coffer::editor;
use coffer::notes::title_from_body;
use coffer::vault::Vault;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let stdin_data = if cli.password_stdin {
        Some(stdin_read_all()?)
    } else {
        None
    };

    match &cli.command {
        Command::Init => cmd_init(&cli, &stdin_data),
        Command::New { title } => cmd_new(&cli, &stdin_data, title.as_deref()),
        Command::List => cmd_list(&cli, &stdin_data),
        Command::Show { query } => cmd_show(&cli, &stdin_data, query),
        Command::Edit { query } => cmd_edit(&cli, &stdin_data, query),
        Command::Rm { query, force } => cmd_rm(&cli, &stdin_data, query, *force),
        Command::Search { term } => cmd_search(&cli, &stdin_data, term),
        Command::Passwd => cmd_passwd(&cli, &stdin_data),
    }
}

fn cmd_init(cli: &Cli, stdin_data: &Option<String>) -> Result<()> {
    let password = get_password(cli, stdin_data, true)?;
    let path = vault_path(cli)?;
    Vault::init(&path, password.as_bytes())?;
    println!("vault created at {}", path.display());
    Ok(())
}

fn cmd_new(cli: &Cli, stdin_data: &Option<String>, title: Option<&str>) -> Result<()> {
    let password = get_password(cli, stdin_data, false)?;
    let path = vault_path(cli)?;
    let mut vault = Vault::open(&path, password.as_bytes())?;

    let body = if let Some(rest) = stdin_body(stdin_data) {
        if !rest.trim().is_empty() {
            rest.trim_end_matches('\n').to_string()
        } else if tty_available() {
            editor::edit("")?
        } else {
            bail!("empty note, nothing saved");
        }
    } else if !io::stdin().is_terminal() {
        let mut body = String::new();
        io::stdin().read_to_string(&mut body)?;
        body.trim_end_matches('\n').to_string()
    } else {
        editor::edit("")?
    };

    if body.trim().is_empty() {
        bail!("empty note, nothing saved");
    }
    let title = match title {
        Some(title) => title.to_string(),
        None => title_from_body(&body),
    };
    let id = vault.notebook_mut().add(&title, &body);
    vault.save(password.as_bytes())?;
    println!("added note {id}");
    Ok(())
}

fn cmd_list(cli: &Cli, stdin_data: &Option<String>) -> Result<()> {
    let password = get_password(cli, stdin_data, false)?;
    let path = vault_path(cli)?;
    let vault = Vault::open(&path, password.as_bytes())?;
    let notes = &vault.notebook().notes;
    if notes.is_empty() {
        println!("vault is empty");
        return Ok(());
    }
    for note in notes {
        let updated = note.updated.get(..19).unwrap_or(&note.updated);
        println!("{:>4}  {}  {}", note.id, updated, note.title);
    }
    Ok(())
}

fn cmd_show(cli: &Cli, stdin_data: &Option<String>, query: &str) -> Result<()> {
    let password = get_password(cli, stdin_data, false)?;
    let path = vault_path(cli)?;
    let vault = Vault::open(&path, password.as_bytes())?;
    let id = vault.notebook().resolve(query)?;
    let note = vault
        .notebook()
        .get(id)
        .context("note disappeared from the vault")?;
    println!("{}", note.body);
    Ok(())
}

fn cmd_edit(cli: &Cli, stdin_data: &Option<String>, query: &str) -> Result<()> {
    let password = get_password(cli, stdin_data, false)?;
    let path = vault_path(cli)?;
    let mut vault = Vault::open(&path, password.as_bytes())?;
    let id = vault.notebook().resolve(query)?;
    let note = vault
        .notebook()
        .get(id)
        .context("note disappeared from the vault")?;
    let title = note.title.clone();
    let old_body = note.body.clone();

    let new_body = editor::edit(&old_body)?;
    if new_body == old_body {
        println!("no changes");
        return Ok(());
    }
    vault.notebook_mut().update(id, &title, &new_body)?;
    vault.save(password.as_bytes())?;
    println!("updated note {id}");
    Ok(())
}

fn cmd_rm(cli: &Cli, stdin_data: &Option<String>, query: &str, force: bool) -> Result<()> {
    let password = get_password(cli, stdin_data, false)?;
    let path = vault_path(cli)?;
    let mut vault = Vault::open(&path, password.as_bytes())?;
    let id = vault.notebook().resolve(query)?;
    let title = vault
        .notebook()
        .get(id)
        .context("note disappeared from the vault")?
        .title
        .clone();

    if !force {
        if !io::stdin().is_terminal() {
            bail!("refusing to delete '{title}' without --force on a non-interactive stdin");
        }
        if !confirm(&format!("Delete '{title}'?"))? {
            println!("aborted");
            return Ok(());
        }
    }
    vault.notebook_mut().remove(id)?;
    vault.save(password.as_bytes())?;
    println!("deleted note {id}");
    Ok(())
}

fn cmd_search(cli: &Cli, stdin_data: &Option<String>, term: &str) -> Result<()> {
    let password = get_password(cli, stdin_data, false)?;
    let path = vault_path(cli)?;
    let vault = Vault::open(&path, password.as_bytes())?;
    let matches = vault.notebook().search(term);
    if matches.is_empty() {
        println!("no matches");
        return Ok(());
    }
    let needle = term.to_lowercase();
    for note in matches {
        println!("[{}] {}", note.id, note.title);
        for line in note.body.lines() {
            if line.to_lowercase().contains(&needle) {
                println!("  {}", line.trim());
            }
        }
    }
    Ok(())
}

fn cmd_passwd(cli: &Cli, stdin_data: &Option<String>) -> Result<()> {
    let old = get_password(cli, stdin_data, false)?;
    let path = vault_path(cli)?;
    let vault = Vault::open(&path, old.as_bytes())?;
    drop(old);
    if !tty_available() {
        bail!("changing the password requires an interactive terminal");
    }
    let new = prompt_password(true)?;
    vault.rekey(new.as_bytes())?;
    println!("password changed");
    Ok(())
}

fn vault_path(cli: &Cli) -> Result<PathBuf> {
    if let Some(path) = &cli.vault {
        return Ok(path.clone());
    }
    let home = dirs::home_dir().context("could not determine the home directory")?;
    Ok(home.join("coffer-vault").join("vault"))
}

fn get_password(
    cli: &Cli,
    stdin_data: &Option<String>,
    confirm: bool,
) -> Result<Zeroizing<String>> {
    if cli.password_stdin {
        let data = stdin_data.as_deref().unwrap_or("");
        let line = data.split('\n').next().unwrap_or("").trim_end_matches('\r');
        if line.is_empty() {
            bail!("password must not be empty");
        }
        warn_if_weak(line);
        Ok(Zeroizing::new(line.to_string()))
    } else {
        prompt_password(confirm)
    }
}

fn stdin_body(stdin_data: &Option<String>) -> Option<String> {
    stdin_data
        .as_deref()
        .map(|data| match data.split_once('\n') {
            Some((_, rest)) => rest.to_string(),
            None => String::new(),
        })
}

fn prompt_password(confirm: bool) -> Result<Zeroizing<String>> {
    let read = || {
        rpassword::prompt_password("Password: ").map_err(|e| {
            anyhow::anyhow!(
                "cannot read the password from the terminal ({e}); use --password-stdin"
            )
        })
    };
    let password = Zeroizing::new(read()?);
    if password.is_empty() {
        bail!("password must not be empty");
    }
    if confirm {
        let again = Zeroizing::new(read()?);
        if *password != *again {
            bail!("passwords do not match");
        }
    }
    warn_if_weak(&password);
    Ok(password)
}

fn warn_if_weak(password: &str) {
    if password.chars().count() < 12 {
        eprintln!(
            "warning: this password is shorter than 12 characters; a long passphrase is recommended"
        );
    }
}

fn tty_available() -> bool {
    std::fs::File::open("/dev/tty")
        .map(|f| f.is_terminal())
        .unwrap_or(false)
}

fn stdin_read_all() -> Result<String> {
    let mut data = String::new();
    io::stdin()
        .read_to_string(&mut data)
        .context("failed to read stdin")?;
    Ok(data)
}

fn confirm(prompt: &str) -> Result<bool> {
    eprint!("{prompt} [y/N] ");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
