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
    Build {
        #[command(flatten)]
        common: Common,
    },
    /// Build configurations and push to targets
    Push {
        #[command(flatten)]
        common: Common,
    },
    /// Apply configurations on remote machines
    Apply {
        #[command(flatten)]
        common: Common,
    },
    /// Show information about the selected flake
    Show {},
    Exec {
        #[command(flatten)]
        common: Common,
        #[arg()]
        command: String,
    },
}

#[derive(Debug, Args, Clone)]
pub struct Common {
    /// Comma-separated list of target hosts
    #[arg(long, value_name = "HOST", value_delimiter = ',', required = true)]
    pub hosts: Vec<String>,
}
