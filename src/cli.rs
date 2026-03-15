use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
    /// Flake ref of the flake to be evaluated
    #[arg(short, long, value_name = "FLAKE REF", default_value = ".")]
    pub flake: String,
}

#[derive(Debug, Subcommand, Clone)]
pub enum Commands {
    /// Only evaluate the specified configurations
    Eval {
        #[command(flatten)]
        common: Common,
    },
    /// Build configurations without pushing to targets
    #[command()]
    Build {
        #[command(flatten)]
        common: Common,
    },
    /// Build configurations and push to targets
    #[command()]
    Push {
        #[command(flatten)]
        common: Common,
    },
    /// Apply configurations on remote machines
    #[command()]
    Apply {
        #[command(flatten)]
        common: Common,
    },
    /// Show information about the selected flake
    #[command()]
    Show {},
}

#[derive(Debug, Args, Clone)]
pub struct Common {
    #[arg(long, value_delimiter = ',', required = true)]
    pub hosts: Vec<String>,
}
