use std::time::Duration;
use tracing::debug;

use crate::errors::{DeployError, Result};
use crate::execution::{CommandExecutor, Executor};
use crate::types::{ExecutionTarget, NixosConfiguration};

/// Handles building Nix derivations
pub struct Builder {
    executor: Executor,
    host_identifier: String,
}

impl Builder {
    pub fn new(build_target: &ExecutionTarget) -> Self {
        Self {
            executor: Executor::from_target(build_target),
            host_identifier: build_target.host_identifier(),
        }
    }

    /// Build a NixOS configuration
    pub async fn build_configuration(
        &self,
        config: &NixosConfiguration,
        timeout: Option<Duration>,
    ) -> Result<String> {
        let drv_path = config.drv_path.as_ref().ok_or_else(|| {
            DeployError::nix_build_failed(
                &config.name,
                None,
                "No derivation path available for building".to_string(),
            )
        })?;

        debug!(
            "Building configuration '{}' on {}",
            config.name, self.host_identifier
        );
        debug!("Building derivation: {}", drv_path);

        let result = self
            .executor
            .execute_and_wait(
                "nix-build",
                &[drv_path],
                timeout.or(Some(Duration::from_secs(1800))), // 30 minute default timeout
            )
            .await
            .map_err(|e| {
                DeployError::nix_build_failed(&config.name, Some(drv_path.clone()), e.to_string())
            })?;

        if result.exit_code != Some(0) {
            return Err(DeployError::nix_build_failed(
                &config.name,
                Some(drv_path.clone()),
                format!("nix-build failed: {}", result.stderr_lines.join("\n")),
            ));
        }

        // nix-build outputs the store path to stdout
        let out_path = result
            .stdout_lines
            .last()
            .ok_or_else(|| {
                DeployError::nix_build_failed(
                    &config.name,
                    Some(drv_path.clone()),
                    "No output path returned from nix-build".to_string(),
                )
            })?
            .trim()
            .to_string();

        debug!(
            "Successfully built configuration '{}': {}",
            config.name, out_path
        );
        Ok(out_path)
    }

    /// Copy a built closure to another host using nix-copy-closure
    pub async fn copy_closure_to(
        &self,
        out_path: &str,
        target: &ExecutionTarget,
        timeout: Option<Duration>,
    ) -> Result<()> {
        if self.executor_matches_target(target) {
            debug!("Skipping closure copy - source and target are the same");
            return Ok(());
        }

        let target_identifier = target.host_identifier();
        debug!(
            "Copying closure {} from {} to {}",
            out_path, self.host_identifier, target_identifier
        );

        let target_spec = match target {
            ExecutionTarget::Local => {
                return Err(DeployError::copy_closure_failed(
                    self.host_identifier.clone(),
                    target_identifier,
                    "Cannot copy closure to local from remote host using nix-copy-closure"
                        .to_string(),
                ));
            }
            ExecutionTarget::Ssh {
                host,
                username,
                port,
            } => {
                let mut spec = if let Some(username) = username {
                    format!("{}@{}", username, host)
                } else {
                    host.clone()
                };
                if let Some(port) = port {
                    spec = format!("ssh://{}:{}", spec, port);
                }
                spec
            }
        };

        let mut args = vec!["--to", &target_spec, out_path];

        // Add SSH options if needed
        if let ExecutionTarget::Ssh {
            port: Some(_port), ..
        } = target
        {
            args.insert(0, "--gzip");
            args.insert(1, "--include-outputs");
        }

        let result = self
            .executor
            .execute_and_wait(
                "nix-copy-closure",
                &args,
                timeout.or(Some(Duration::from_secs(1800))), // 30 minute default timeout
            )
            .await
            .map_err(|e| {
                DeployError::copy_closure_failed(
                    self.host_identifier.clone(),
                    target_identifier.clone(),
                    format!("nix-copy-closure failed: {}", e),
                )
            })?;

        if result.exit_code != Some(0) {
            return Err(DeployError::copy_closure_failed(
                self.host_identifier.clone(),
                target_identifier,
                format!(
                    "nix-copy-closure failed: {}",
                    result.stderr_lines.join("\n")
                ),
            ));
        }

        debug!(
            "Successfully copied closure {} to {}",
            out_path, target_identifier
        );
        Ok(())
    }

    /// Check if this builder's executor matches the given target
    fn executor_matches_target(&self, target: &ExecutionTarget) -> bool {
        match (&self.executor, target) {
            (Executor::Local(_), ExecutionTarget::Local) => true,
            (
                Executor::Ssh(ssh_executor),
                ExecutionTarget::Ssh {
                    host,
                    username,
                    port,
                },
            ) => {
                // This is a simplified check - in practice you might want more sophisticated matching
                ssh_executor.host == *host
                    && ssh_executor.username.as_ref() == username.as_ref()
                    && ssh_executor.port == *port
            }
            _ => false,
        }
    }
}

/// Check if a store path exists on a given target
pub async fn check_store_path_exists(
    target: &ExecutionTarget,
    store_path: &str,
    timeout: Option<Duration>,
) -> Result<bool> {
    let executor = Executor::from_target(target);

    let result = executor
        .execute_and_wait(
            "test",
            &["-e", store_path],
            timeout.or(Some(Duration::from_secs(30))),
        )
        .await;

    match result {
        Ok(cmd_result) => Ok(cmd_result.exit_code == Some(0)),
        Err(_) => Ok(false), // If command fails, assume path doesn't exist
    }
}
