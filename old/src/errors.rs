use std::path::PathBuf;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, DeployError>;

#[derive(Error, Debug)]
pub enum DeployError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Command execution failed: {command}\nError output: {stderr}")]
    CommandFailed {
        command: String,
        exit_code: Option<i32>,
        stderr: String,
    },

    #[error("Nix evaluation failed for flake at {flake_path}: {message}")]
    NixEvaluationFailed {
        flake_path: PathBuf,
        message: String,
    },

    #[error("Nix build failed for configuration {config_name}: {message}")]
    NixBuildFailed {
        config_name: String,
        drv_path: Option<String>,
        message: String,
    },

    #[error("Deployment failed for configuration {config_name}: {message}")]
    DeploymentFailed {
        config_name: String,
        message: String,
    },

    #[error("SSH connection failed to {host}: {message}")]
    SshConnectionFailed { host: String, message: String },

    #[error("Configuration not found: {config_name}")]
    ConfigurationNotFound { config_name: String },

    #[error("Invalid configuration: {message}")]
    InvalidConfiguration { message: String },

    #[error("Copy closure failed from {src} to {target}: {message}")]
    CopyClosureFailed {
        src: String,
        target: String,
        message: String,
    },

    #[error("Timeout occurred during {operation} after {timeout_secs} seconds")]
    Timeout {
        operation: String,
        timeout_secs: u64,
    },

    #[error("Multiple errors occurred")]
    Multiple { errors: Vec<DeployError> },

    #[error("Channel error: {0}")]
    Channel(String),

    #[error("Other error: {0}")]
    Other(String),
}

impl DeployError {
    pub fn command_failed(
        command: impl Into<String>,
        exit_code: Option<i32>,
        stderr: impl Into<String>,
    ) -> Self {
        Self::CommandFailed {
            command: command.into(),
            exit_code,
            stderr: stderr.into(),
        }
    }

    pub fn nix_evaluation_failed(flake_path: PathBuf, message: impl Into<String>) -> Self {
        Self::NixEvaluationFailed {
            flake_path,
            message: message.into(),
        }
    }

    pub fn nix_build_failed(
        config_name: impl Into<String>,
        drv_path: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::NixBuildFailed {
            config_name: config_name.into(),
            drv_path,
            message: message.into(),
        }
    }

    pub fn deployment_failed(config_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self::DeploymentFailed {
            config_name: config_name.into(),
            message: message.into(),
        }
    }

    pub fn ssh_connection_failed(host: impl Into<String>, message: impl Into<String>) -> Self {
        Self::SshConnectionFailed {
            host: host.into(),
            message: message.into(),
        }
    }

    pub fn configuration_not_found(config_name: impl Into<String>) -> Self {
        Self::ConfigurationNotFound {
            config_name: config_name.into(),
        }
    }

    pub fn invalid_configuration(message: impl Into<String>) -> Self {
        Self::InvalidConfiguration {
            message: message.into(),
        }
    }

    pub fn copy_closure_failed(
        source: impl Into<String>,
        target: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::CopyClosureFailed {
            src: source.into(),
            target: target.into(),
            message: message.into(),
        }
    }

    pub fn timeout(operation: impl Into<String>, timeout_secs: u64) -> Self {
        Self::Timeout {
            operation: operation.into(),
            timeout_secs,
        }
    }

    pub fn multiple(errors: Vec<DeployError>) -> Self {
        Self::Multiple { errors }
    }

    pub fn channel(message: impl Into<String>) -> Self {
        Self::Channel(message.into())
    }

    /// Get the last N lines from stderr for error display
    pub fn get_error_tail(&self, lines: usize) -> Vec<String> {
        match self {
            Self::CommandFailed { stderr, .. } => stderr
                .lines()
                .rev()
                .take(lines)
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect(),
            Self::NixBuildFailed { message, .. }
            | Self::DeploymentFailed { message, .. }
            | Self::SshConnectionFailed { message, .. } => message
                .lines()
                .rev()
                .take(lines)
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect(),
            _ => vec![self.to_string()],
        }
    }
}

/// Collect multiple results, continuing on error and collecting all errors
pub fn collect_results<T>(results: Vec<Result<T>>) -> Result<Vec<T>> {
    let mut successes = Vec::new();
    let mut errors = Vec::new();

    for result in results {
        match result {
            Ok(value) => successes.push(value),
            Err(error) => errors.push(error),
        }
    }

    if errors.is_empty() {
        Ok(successes)
    } else if successes.is_empty() {
        Err(if errors.len() == 1 {
            errors.into_iter().next().unwrap()
        } else {
            DeployError::multiple(errors)
        })
    } else {
        // Some succeeded, some failed - still return error but with partial results
        Err(DeployError::multiple(errors))
    }
}
