use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "pkgs")]
#[command(about = "pkgs - Linux package search engine. https://pkgs.org")]
#[command(version = env!("VERSION"))]
pub struct Cli {
    /// Path to one or more config files (merged in order).
    #[arg(long, default_value = "config.toml", action = clap::ArgAction::Append)]
    pub config: Vec<PathBuf>,

    /// Path to SQLite database file.
    #[arg(long = "db", default_value = "data.db")]
    pub db_path: PathBuf,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
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
