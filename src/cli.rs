use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "coffer",
    version,
    about = "Encrypted, text-only notes in a single vault file"
)]
pub struct Cli {
    /// Path to the vault file
    #[arg(long, env = "COFFER_VAULT", global = true, value_name = "PATH")]
    pub vault: Option<PathBuf>,

    /// Read the password from the first line of stdin
    #[arg(long, global = true)]
    pub password_stdin: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a new vault
    Init,
    /// Add a note (opens $EDITOR, or reads a piped stdin)
    New {
        /// Note title (defaults to the first line of the body)
        #[arg(short, long)]
        title: Option<String>,
    },
    /// List notes
    List,
    /// Show a note
    Show {
        /// Note id or exact title
        query: String,
    },
    /// Edit a note
    Edit {
        /// Note id or exact title
        query: String,
    },
    /// Delete a note
    Rm {
        /// Note id or exact title
        query: String,
        /// Do not ask for confirmation
        #[arg(short, long)]
        force: bool,
    },
    /// Search note titles and bodies
    Search {
        /// Text to search for (case-insensitive)
        term: String,
    },
    /// Change the master password
    Passwd,
}
