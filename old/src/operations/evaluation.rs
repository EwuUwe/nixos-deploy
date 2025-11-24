use std::{collections::HashMap, path::PathBuf, time::Duration};

use async_stream::try_stream;
use serde_json;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio_stream::Stream;
use tracing::debug;

use crate::errors::{DeployError, Result};
use crate::execution::{CommandExecutor, Executor};
use crate::types::{ConfigurationMetadata, ExecutionTarget, NixEvalJobOutput};

/// Handles Nix expression evaluation
pub struct Evaluator {
    flake_path: PathBuf,
    executor: Executor,
}

impl Evaluator {
    pub fn new(flake_path: PathBuf, evaluation_target: &ExecutionTarget) -> Self {
        Self {
            flake_path,
            executor: Executor::from_target(evaluation_target),
        }
    }

    /// Evaluate deployment configuration metadata for all NixOS configurations
    pub async fn evaluate_deployment_metadata(&self) -> Result<HashMap<String, ConfigurationMetadata>> {
        debug!("Evaluating deployment metadata for flake at {:?}", self.flake_path);

        let eval_expr = format!(
            "path:{}#nixosConfigurations",
            self.flake_path.to_string_lossy()
        );

        let result = self.executor.execute_and_wait(
            "nix",
            &[
                "eval",
                "--json",
                &eval_expr,
                "--apply",
                "builtins.mapAttrs (_: value: value.config.deploy)",
            ],
            Some(Duration::from_secs(300)), // 5 minute timeout for evaluation
        ).await.map_err(|e| {
            DeployError::nix_evaluation_failed(
                self.flake_path.clone(),
                format!("Failed to evaluate deployment metadata: {}", e),
            )
        })?;

        if result.exit_code != Some(0) {
            return Err(DeployError::nix_evaluation_failed(
                self.flake_path.clone(),
                format!("nix eval failed: {}", result.stderr_lines.join("\n")),
            ));
        }

        let stdout = result.stdout_lines.join("\n");
        let metadata: HashMap<String, ConfigurationMetadata> = serde_json::from_str(&stdout)
            .map_err(|e| DeployError::nix_evaluation_failed(
                self.flake_path.clone(),
                format!("Failed to parse deployment metadata JSON: {}", e),
            ))?;

        Ok(metadata)
    }

    /// Evaluate all NixOS configurations and return a stream of results
    pub async fn evaluate_configurations(
        &self,
    ) -> Result<impl Stream<Item = Result<NixEvalJobOutput>> + Send> {
        debug!("Starting configuration evaluation for flake at {:?}", self.flake_path);

        let eval_expr = format!(
            r#"
            with builtins;
            let flake = getFlake "path:{}";
            in mapAttrs
                (name: config: config.config.system.build.toplevel)
                flake.outputs.nixosConfigurations
            "#,
            self.flake_path.to_string_lossy()
        );

        // For local execution, use nix-eval-jobs directly
        // For remote execution, we need to ensure nix-eval-jobs is available
        let mut child = if self.executor_is_local() {
            Command::new("nix-eval-jobs")
                .arg("--workers")
                .arg("12")
                .arg("--expr")
                .arg(&eval_expr)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| DeployError::nix_evaluation_failed(
                    self.flake_path.clone(),
                    format!("Failed to start nix-eval-jobs: {}", e),
                ))?
        } else {
            // For SSH, we need to run nix-eval-jobs on the remote host
            // This is a simplified implementation - in practice you might want to
            // copy the flake or use different approaches
            return Err(DeployError::nix_evaluation_failed(
                self.flake_path.clone(),
                "Remote evaluation not yet implemented - please use local evaluation".to_string(),
            ));
        };

        let stdout = child.stdout.take().ok_or_else(|| {
            DeployError::nix_evaluation_failed(
                self.flake_path.clone(),
                "Failed to get stdout from nix-eval-jobs".to_string(),
            )
        })?;

        let stderr = child.stderr.take().ok_or_else(|| {
            DeployError::nix_evaluation_failed(
                self.flake_path.clone(),
                "Failed to get stderr from nix-eval-jobs".to_string(),
            )
        })?;

        let stdout_reader = BufReader::new(stdout);
        let stderr_reader = BufReader::new(stderr);

        let mut stdout_lines = stdout_reader.lines();
        let mut stderr_lines = stderr_reader.lines();

        Ok(try_stream! {
            loop {
                tokio::select! {
                    line = stdout_lines.next_line() => {
                        match line {
                            Ok(Some(line)) => {
                                if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&line) {
                                    match serde_json::from_value::<NixEvalJobOutput>(json_value) {
                                        Ok(output) => yield output,
                                        Err(e) => {
                                            debug!("Failed to parse evaluation output: {} - line was: {}", e, line);
                                            // Continue processing other lines
                                        }
                                    }
                                }
                            },
                            Ok(None) => break,
                            Err(e) => {
                                yield Err(DeployError::nix_evaluation_failed(
                                    self.flake_path.clone(),
                                    format!("Error reading stdout: {}", e),
                                ))?;
                            }
                        }
                    }
                    line = stderr_lines.next_line() => {
                        if let Ok(Some(error_line)) = line {
                            if !error_line.trim().is_empty() {
                                debug!("nix-eval-jobs stderr: {}", error_line);
                            }
                        }
                    }
                }
            }

            // Wait for the child process to complete
            let exit_status = child.wait().await.map_err(|e| DeployError::nix_evaluation_failed(
                self.flake_path.clone(),
                format!("Failed to wait for nix-eval-jobs: {}", e),
            ))?;

            if !exit_status.success() {
                yield Err(DeployError::nix_evaluation_failed(
                    self.flake_path.clone(),
                    format!("nix-eval-jobs failed with exit code: {:?}", exit_status.code()),
                ))?;
            }
        })
    }

    /// Copy the flake to a remote evaluation host
    pub async fn copy_flake_to_remote(&self, target: &ExecutionTarget) -> Result<PathBuf> {
        // This is a placeholder for future implementation
        // In practice, you might want to:
        // 1. Create a temporary directory on the remote host
        // 2. Use rsync or nix copy to transfer the flake
        // 3. Return the remote path

        match target {
            ExecutionTarget::Local => Ok(self.flake_path.clone()),
            ExecutionTarget::Ssh { host, .. } => {
                Err(DeployError::nix_evaluation_failed(
                    self.flake_path.clone(),
                    format!("Copying flake to remote host {} not yet implemented", host),
                ))
            }
        }
    }

    fn executor_is_local(&self) -> bool {
        matches!(self.executor, Executor::Local(_))
    }
}
