use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use crate::types::{DeploymentOptions, ExecutionTarget, Verbosity};

#[derive(Parser)]
#[command(version, about = "Deploy NixOS configurations")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Build and deploy NixOS configurations
    Build {
        /// Flake reference path
        #[arg(short, long, value_name = "FLAKE_REF")]
        flake: Option<PathBuf>,

        /// Perform a dry run (use 'test' instead of 'switch')
        #[arg(long)]
        dry_run: bool,

        /// Override evaluation host for all configurations
        #[arg(long, value_name = "HOST")]
        eval_host: Option<String>,

        /// Override build host for all configurations
        #[arg(long, value_name = "HOST")]
        build_host: Option<String>,

        /// Verbosity level
        #[arg(short, long, value_enum, default_value = "progress")]
        verbosity: VerbosityArg,

        /// Maximum number of concurrent operations
        #[arg(long, value_name = "COUNT")]
        max_concurrent: Option<usize>,

        /// Timeout for operations in seconds
        #[arg(long, value_name = "SECONDS")]
        timeout: Option<u64>,

        /// Specific configurations to deploy (deploy all if not specified)
        #[arg(value_name = "CONFIG")]
        configs: Vec<String>,
    },
}

#[derive(Debug, Clone, ValueEnum)]
pub enum VerbosityArg {
    /// Show progress bars with summary information
    Progress,
    /// Show all command output with host prefixes
    Detailed,
}

impl From<VerbosityArg> for Verbosity {
    fn from(arg: VerbosityArg) -> Self {
        match arg {
            VerbosityArg::Progress => Verbosity::Progress,
            VerbosityArg::Detailed => Verbosity::Detailed,
        }
    }
}

impl Cli {
    pub fn parse_deployment_options(&self) -> Option<DeploymentOptions> {
        match &self.command {
            Some(Commands::Build {
                dry_run,
                eval_host,
                build_host,
                verbosity,
                max_concurrent,
                timeout,
                ..
            }) => {
                let eval_host_target = eval_host.as_ref().map(|host| parse_host_spec(host));

                let build_host_target = build_host.as_ref().map(|host| parse_host_spec(host));

                Some(DeploymentOptions {
                    evaluation_host_override: eval_host_target,
                    build_host_override: build_host_target,
                    dry_run: *dry_run,
                    verbosity: verbosity.clone().into(),
                    max_concurrent: *max_concurrent,
                    timeout_secs: *timeout,
                })
            }
            None => None,
        }
    }
}

/// Parse a host specification into an ExecutionTarget
/// Formats supported:
/// - "local" -> Local execution
/// - "hostname" -> SSH to hostname
/// - "user@hostname" -> SSH with specific user
/// - "hostname:port" -> SSH with specific port
/// - "user@hostname:port" -> SSH with user and port
fn parse_host_spec(spec: &str) -> ExecutionTarget {
    if spec == "local" {
        return ExecutionTarget::Local;
    }

    let mut parts = spec.split('@');
    let (username, host_port) = match (parts.next(), parts.next()) {
        (Some(host_port), None) => (None, host_port),
        (Some(user), Some(host_port)) => (Some(user.to_string()), host_port),
        _ => (None, spec),
    };

    let mut host_port_parts = host_port.split(':');
    let host = host_port_parts.next().unwrap_or(spec).to_string();
    let port = host_port_parts.next().and_then(|p| p.parse().ok());

    ExecutionTarget::Ssh {
        host,
        username,
        port,
    }
}
