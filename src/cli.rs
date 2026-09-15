use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "pkgs")]
#[command(about = "pkgs - Linux package search engine. https://pkgs.org")]
#[command(version = env!("VERSION"))]
pub struct Cli {
    /// Path to one or more config files (merged in order).
    #[arg(long, default_value = "config.toml", global = true, action = clap::ArgAction::Append)]
    pub config: Vec<PathBuf>,

    /// Path to SQLite database file.
    #[arg(long = "db", default_value = "data.db", global = true)]
    pub db_path: PathBuf,

    /// Path to the site directory. If empty, only the HTTP API `/api/*`handlers are registered.
    #[arg(long)]
    pub site: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Restore only the required tables form Repology SQL dump streamed to stdin.
    RestoreRepology {
        #[arg(long, default_value = "postgres://repology@127.0.0.1:55432/repology")]
        pg: String,
    },

    /// Import Repology PG dump into a new SQLite database.
    Import {
        #[arg(long, default_value = "postgres://repology@127.0.0.1:55432/repology")]
        pg: String,
    },

    /// Generate a sample config file.
    NewConfig {
        /// Output path for config file.
        #[arg(short, long, default_value = "config.toml")]
        path: PathBuf,
    },

    /// Run first time DB installation.
    Install {
        /// Assume 'yes' to any manual prompts during installation.
        #[arg(long)]
        yes: bool,
    },
}
