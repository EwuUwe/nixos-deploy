use serde::{Deserialize, Serialize};

/// Configuration for where operations should be performed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentConfig {
    /// Where to evaluate the Nix expressions
    pub evaluation_host: Option<ExecutionTarget>,
    /// Where to build the Nix derivations
    pub build_host: Option<ExecutionTarget>,
    /// The target host for deployment
    pub target_host: ExecutionTarget,
    /// Whether this is a dry run (use "test" instead of "switch")
    pub dry_run: bool,
    /// Timeout for operations in seconds
    pub timeout_secs: Option<u64>,
}

/// Represents where an operation should be executed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecutionTarget {
    /// Execute locally
    Local,
    /// Execute via SSH on a remote host
    Ssh {
        host: String,
        username: Option<String>,
        port: Option<u16>,
    },
}

/// Metadata for a NixOS configuration
#[derive(Debug, Clone, Deserialize)]
pub struct ConfigurationMetadata {
    pub system: String,
    pub ips: serde_json::Value,
    pub tags: Vec<String>,
    /// Optional deployment configuration overrides
    pub deploy: Option<DeploymentConfig>,
}

/// A NixOS configuration with all its metadata and paths
#[derive(Debug, Clone)]
pub struct NixosConfiguration {
    pub name: String,
    pub metadata: ConfigurationMetadata,
    pub deployment_config: DeploymentConfig,
    pub drv_path: Option<String>,
    pub out_path: Option<String>,
}

/// Result of a Nix evaluation job
#[derive(Debug, Clone, Deserialize)]
pub struct NixEvalJobOutput {
    #[serde(rename = "drvPath")]
    pub drv_path: Option<String>,
    pub outputs: Option<NixOutputs>,
    pub attr: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub system: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NixOutputs {
    pub out: String,
}

/// Status updates for a configuration during deployment
#[derive(Debug, Clone)]
pub enum ConfigurationStatus {
    Started(String),
    Evaluating,
    Evaluated {
        drv_path: String,
    },
    Building,
    Built {
        result: String,
    },
    CopyingClosure,
    Activating,
    Activated,
    Error {
        error: String,
        error_lines: Vec<String>,
    },
}

/// Verbosity level for output
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verbosity {
    /// Show progress bars with summary information
    Progress,
    /// Show all command output with host prefixes
    Detailed,
}

/// CLI options that affect deployment behavior
#[derive(Debug, Clone)]
pub struct DeploymentOptions {
    /// Override evaluation host for all configurations
    pub evaluation_host_override: Option<ExecutionTarget>,
    /// Override build host for all configurations
    pub build_host_override: Option<ExecutionTarget>,
    /// Whether to perform a dry run
    pub dry_run: bool,
    /// Verbosity level
    pub verbosity: Verbosity,
    /// Maximum number of concurrent operations
    pub max_concurrent: Option<usize>,
    /// Timeout for operations
    pub timeout_secs: Option<u64>,
}

impl Default for DeploymentOptions {
    fn default() -> Self {
        Self {
            evaluation_host_override: None,
            build_host_override: None,
            dry_run: false,
            verbosity: Verbosity::Progress,
            max_concurrent: None,
            timeout_secs: Some(600), // 10 minutes default
        }
    }
}

impl ExecutionTarget {
    pub fn is_local(&self) -> bool {
        matches!(self, ExecutionTarget::Local)
    }

    pub fn host_identifier(&self) -> String {
        match self {
            ExecutionTarget::Local => "local".to_string(),
            ExecutionTarget::Ssh {
                host,
                username,
                port,
            } => {
                let mut identifier = if let Some(username) = username {
                    format!("{}@{}", username, host)
                } else {
                    host.clone()
                };
                if let Some(port) = port {
                    identifier.push_str(&format!(":{}", port));
                }
                identifier
            }
        }
    }
}

impl DeploymentConfig {
    pub fn new(target_host: ExecutionTarget) -> Self {
        Self {
            evaluation_host: None,
            build_host: None,
            target_host,
            dry_run: false,
            timeout_secs: Some(600),
        }
    }

    /// Apply CLI overrides to this configuration
    pub fn apply_overrides(&mut self, options: &DeploymentOptions) {
        if let Some(ref eval_host) = options.evaluation_host_override {
            self.evaluation_host = Some(eval_host.clone());
        }
        if let Some(ref build_host) = options.build_host_override {
            self.build_host = Some(build_host.clone());
        }
        if options.dry_run {
            self.dry_run = true;
        }
        if let Some(timeout) = options.timeout_secs {
            self.timeout_secs = Some(timeout);
        }
    }

    /// Get the effective evaluation host (fallback to target if not specified)
    pub fn effective_evaluation_host(&self) -> &ExecutionTarget {
        self.evaluation_host.as_ref().unwrap_or(&self.target_host)
    }

    /// Get the effective build host (fallback to evaluation host if not specified)
    pub fn effective_build_host(&self) -> &ExecutionTarget {
        self.build_host
            .as_ref()
            .unwrap_or_else(|| self.effective_evaluation_host())
    }
}

impl NixosConfiguration {
    pub fn new(name: String, metadata: ConfigurationMetadata) -> crate::errors::Result<Self> {
        // Extract primary IP for SSH target
        let primary_ip = metadata
            .ips
            .get("primary")
            .and_then(|ip| ip.as_str())
            .ok_or_else(|| {
                crate::errors::DeployError::invalid_configuration(format!(
                    "Configuration '{}' missing primary IP",
                    name
                ))
            })?;

        let target_host = ExecutionTarget::Ssh {
            host: primary_ip.to_string(),
            username: None,
            port: None,
        };

        let deployment_config = metadata
            .deploy
            .clone()
            .unwrap_or_else(|| DeploymentConfig::new(target_host.clone()));

        Ok(Self {
            name,
            metadata,
            deployment_config,
            drv_path: None,
            out_path: None,
        })
    }

    pub fn with_paths(mut self, drv_path: Option<String>, out_path: Option<String>) -> Self {
        self.drv_path = drv_path;
        self.out_path = out_path;
        self
    }
}
