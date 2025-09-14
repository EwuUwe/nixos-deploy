use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Build NixOS configurations
    Build {
        /// Flake reference path
        #[arg(short, long, value_name = "flake ref")]
        flake: Option<PathBuf>,
    },
}
