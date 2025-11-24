use std::time::Duration;
use tracing::debug;

use crate::errors::{DeployError, Result};
use crate::execution::{CommandExecutor, Executor};
use crate::types::{ExecutionTarget, NixosConfiguration, Verbosity};

/// Handles deployment operations on target hosts
pub struct Deployer {
    executor: Executor,
    host_identifier: String,
}

impl Deployer {
    pub fn new(target: &ExecutionTarget) -> Self {
        Self {
            executor: Executor::from_target(target),
            host_identifier: target.host_identifier(),
        }
    }

    /// Activate a NixOS configuration on the target host
    pub async fn activate_configuration(
        &self,
        config: &NixosConfiguration,
        verbosity: Verbosity,
        timeout: Option<Duration>,
    ) -> Result<Vec<String>> {
        let out_path = config.out_path.as_ref().ok_or_else(|| {
            DeployError::deployment_failed(
                &config.name,
                "No output path available for activation".to_string(),
            )
        })?;

        let switch_command = if config.deployment_config.dry_run {
            "test"
        } else {
            "switch"
        };

        let activation_script = format!("{}/bin/switch-to-configuration", out_path);

        debug!(
            "Activating configuration '{}' on {} (mode: {})",
            config.name, self.host_identifier, switch_command
        );

        // Use streaming execution to capture real-time output
        let mut handle = self
            .executor
            .execute_streaming(
                &activation_script,
                &[switch_command],
                &config.name,
                verbosity,
            )
            .await
            .map_err(|e| {
                DeployError::deployment_failed(
                    &config.name,
                    format!("Failed to start activation: {}", e),
                )
            })?;

        let mut error_lines = Vec::new();
        let mut output_lines = Vec::new();

        // Collect output
        loop {
            tokio::select! {
                stdout_line = handle.stdout_receiver.recv() => {
                    match stdout_line {
                        Some(line) => {
                            debug!("Activation stdout: {}", line);
                            output_lines.push(line);
                        }
                        None => break,
                    }
                }
                stderr_line = handle.stderr_receiver.recv() => {
                    match stderr_line {
                        Some(line) => {
                            // Store stderr for error reporting but don't print to avoid
                            // interfering with progress bars - full details in final summary
                            error_lines.push(line);
                        }
                        None => {}
                    }
                }
            }
        }

        // Wait for the process to complete
        let exit_status = tokio::time::timeout(
            timeout.unwrap_or(Duration::from_secs(600)), // 10 minute default
            handle.child.wait(),
        )
        .await
        .map_err(|_| {
            DeployError::timeout(
                format!("activation of {}", config.name),
                timeout.unwrap_or(Duration::from_secs(600)).as_secs(),
            )
        })?
        .map_err(|e| {
            DeployError::deployment_failed(
                &config.name,
                format!("Failed to wait for activation process: {}", e),
            )
        })?;

        if !exit_status.success() {
            return Err(DeployError::deployment_failed(
                &config.name,
                format!(
                    "Activation failed with exit code {:?}. Last {} error lines:\n{}",
                    exit_status.code(),
                    std::cmp::min(error_lines.len(), 10),
                    error_lines
                        .iter()
                        .rev()
                        .take(10)
                        .cloned()
                        .collect::<Vec<_>>()
                        .iter()
                        .rev()
                        .cloned()
                        .collect::<Vec<String>>()
                        .join("\n")
                ),
            ));
        }

        if config.deployment_config.dry_run {
            debug!(
                "Dry run completed successfully for configuration '{}'",
                config.name
            );
        } else {
            debug!("Successfully activated configuration '{}'", config.name);
        }

        Ok(error_lines)
    }

    /// Check the health of a deployed configuration
    /// This is a placeholder for future health check implementation
    pub async fn check_health(
        &self,
        config: &NixosConfiguration,
        timeout: Option<Duration>,
    ) -> Result<HealthStatus> {
        debug!(
            "Checking health for configuration '{}' on {}",
            config.name, self.host_identifier
        );

        // Basic health check - verify systemctl status
        let result = self
            .executor
            .execute_and_wait(
                "systemctl",
                &["is-system-running"],
                timeout.or(Some(Duration::from_secs(30))),
            )
            .await;

        match result {
            Ok(cmd_result) => {
                if cmd_result.exit_code == Some(0) {
                    let status = cmd_result
                        .stdout_lines
                        .first()
                        .map(|s| s.trim().to_string())
                        .unwrap_or_else(|| "unknown".to_string());

                    match status.as_str() {
                        "running" => Ok(HealthStatus::Healthy),
                        "degraded" => Ok(HealthStatus::Degraded {
                            reason: "System is degraded".to_string(),
                        }),
                        _ => Ok(HealthStatus::Unknown { status }),
                    }
                } else {
                    Ok(HealthStatus::Unhealthy {
                        reason: format!("systemctl failed: {}", cmd_result.stderr_lines.join("\n")),
                    })
                }
            }
            Err(e) => Ok(HealthStatus::Unhealthy {
                reason: format!("Health check failed: {}", e),
            }),
        }
    }

    /// Prepare for rollback by storing current configuration information
    /// This is a placeholder for future rollback implementation
    pub async fn prepare_rollback(&self, config: &NixosConfiguration) -> Result<RollbackInfo> {
        debug!(
            "Preparing rollback info for configuration '{}' on {}",
            config.name, self.host_identifier
        );

        // Get current system profile
        let result = self
            .executor
            .execute_and_wait(
                "readlink",
                &["/nix/var/nix/profiles/system"],
                Some(Duration::from_secs(30)),
            )
            .await
            .map_err(|e| {
                DeployError::deployment_failed(
                    &config.name,
                    format!("Failed to get current system profile: {}", e),
                )
            })?;

        let current_profile = result
            .stdout_lines
            .first()
            .ok_or_else(|| {
                DeployError::deployment_failed(
                    &config.name,
                    "No current system profile found".to_string(),
                )
            })?
            .trim()
            .to_string();

        Ok(RollbackInfo {
            config_name: config.name.clone(),
            previous_profile: current_profile,
            host: self.host_identifier.clone(),
        })
    }
}

/// Health status of a deployed system
#[derive(Debug, Clone)]
pub enum HealthStatus {
    Healthy,
    Degraded { reason: String },
    Unhealthy { reason: String },
    Unknown { status: String },
}

/// Information needed for rollback operations
#[derive(Debug, Clone)]
pub struct RollbackInfo {
    pub config_name: String,
    pub previous_profile: String,
    pub host: String,
}

impl RollbackInfo {
    /// Execute a rollback to the previous configuration
    /// This is a placeholder for future rollback implementation
    pub async fn execute_rollback(
        &self,
        target: &ExecutionTarget,
        timeout: Option<Duration>,
    ) -> Result<()> {
        debug!(
            "Rolling back configuration '{}' on {}",
            self.config_name, self.host
        );

        let executor = Executor::from_target(target);

        // Activate the previous profile
        let activation_script = format!("{}/bin/switch-to-configuration", self.previous_profile);

        let result = executor
            .execute_and_wait(
                &activation_script,
                &["switch"],
                timeout.or(Some(Duration::from_secs(600))),
            )
            .await
            .map_err(|e| {
                DeployError::deployment_failed(&self.config_name, format!("Rollback failed: {}", e))
            })?;

        if result.exit_code != Some(0) {
            return Err(DeployError::deployment_failed(
                &self.config_name,
                format!(
                    "Rollback activation failed: {}",
                    result.stderr_lines.join("\n")
                ),
            ));
        }

        debug!(
            "Successfully rolled back configuration '{}' on {}",
            self.config_name, self.host
        );
        Ok(())
    }
}
