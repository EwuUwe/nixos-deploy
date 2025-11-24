use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::debug;

use crate::errors::{DeployError, Result};
use crate::types::{ExecutionTarget, Verbosity};

/// Result of executing a command with streaming output
pub struct CommandResult {
    pub exit_code: Option<i32>,
    pub stdout_lines: Vec<String>,
    pub stderr_lines: Vec<String>,
}

/// Handle for a running command that provides streaming output
pub struct CommandHandle {
    pub child: Child,
    pub stdout_receiver: mpsc::Receiver<String>,
    pub stderr_receiver: mpsc::Receiver<String>,
}

#[async_trait]
pub trait CommandExecutor: Send + Sync {
    /// Execute a command and return immediately with a handle for streaming output
    async fn execute_streaming(
        &self,
        program: &str,
        args: &[&str],
        host_prefix: &str,
        verbosity: Verbosity,
    ) -> Result<CommandHandle>;

    /// Execute a command and wait for completion, collecting all output
    async fn execute_and_wait(
        &self,
        program: &str,
        args: &[&str],
        timeout_duration: Option<Duration>,
    ) -> Result<CommandResult>;
}

/// Local command executor
#[derive(Debug, Clone)]
pub struct LocalExecutor;

impl LocalExecutor {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl CommandExecutor for LocalExecutor {
    async fn execute_streaming(
        &self,
        program: &str,
        args: &[&str],
        host_prefix: &str,
        verbosity: Verbosity,
    ) -> Result<CommandHandle> {
        debug!("Executing locally: {} {}", program, args.join(" "));

        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn()?;

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let (stdout_tx, stdout_rx) = mpsc::channel(100);
        let (stderr_tx, stderr_rx) = mpsc::channel(100);

        // Spawn task to read stdout
        let stdout_prefix = format!("[{}]", host_prefix);
        let stdout_verbosity = verbosity;
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                match stdout_verbosity {
                    Verbosity::Progress => {
                        // Only send important lines or summaries
                        if line.contains("error") || line.contains("warning") || line.len() > 100 {
                            let _ = stdout_tx.send(format!("{} {}", stdout_prefix, line)).await;
                        }
                    }
                    Verbosity::Detailed => {
                        let _ = stdout_tx.send(format!("{} {}", stdout_prefix, line)).await;
                    }
                }
            }
        });

        // Spawn task to read stderr
        let stderr_prefix = format!("[{}]", host_prefix);
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                // Send stderr lines with host prefix but without automatic ERROR: prefix
                let _ = stderr_tx.send(format!("{} {}", stderr_prefix, line)).await;
            }
        });

        Ok(CommandHandle {
            child,
            stdout_receiver: stdout_rx,
            stderr_receiver: stderr_rx,
        })
    }

    async fn execute_and_wait(
        &self,
        program: &str,
        args: &[&str],
        timeout_duration: Option<Duration>,
    ) -> Result<CommandResult> {
        debug!(
            "Executing locally and waiting: {} {}",
            program,
            args.join(" ")
        );

        let mut cmd = Command::new(program);
        cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

        let child = cmd.spawn()?;

        let execution = async {
            let output = child.wait_with_output().await?;

            let stdout_lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|s| s.to_string())
                .collect();

            let stderr_lines: Vec<String> = String::from_utf8_lossy(&output.stderr)
                .lines()
                .map(|s| s.to_string())
                .collect();

            if !output.status.success() {
                return Err(DeployError::command_failed(
                    format!("{} {}", program, args.join(" ")),
                    output.status.code(),
                    stderr_lines.join("\n"),
                ));
            }

            Ok(CommandResult {
                exit_code: output.status.code(),
                stdout_lines,
                stderr_lines,
            })
        };

        if let Some(duration) = timeout_duration {
            timeout(duration, execution).await.map_err(|_| {
                DeployError::timeout(
                    format!("{} {}", program, args.join(" ")),
                    duration.as_secs(),
                )
            })?
        } else {
            execution.await
        }
    }
}

/// SSH command executor
#[derive(Debug, Clone)]
pub struct SshExecutor {
    pub host: String,
    pub username: Option<String>,
    pub port: Option<u16>,
}

impl SshExecutor {
    pub fn new(host: String) -> Self {
        Self {
            host,
            username: None,
            port: None,
        }
    }

    pub fn with_username(mut self, username: String) -> Self {
        self.username = Some(username);
        self
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    fn build_ssh_args(&self) -> Vec<String> {
        let mut ssh_args = Vec::new();

        // Add common SSH options for automation
        ssh_args.extend_from_slice(&[
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            "-o".to_string(),
            "StrictHostKeyChecking=no".to_string(),
            "-o".to_string(),
            "UserKnownHostsFile=/dev/null".to_string(),
        ]);

        if let Some(port) = self.port {
            ssh_args.push("-p".to_string());
            ssh_args.push(port.to_string());
        }

        if let Some(username) = &self.username {
            ssh_args.push(format!("{}@{}", username, self.host));
        } else {
            ssh_args.push(self.host.clone());
        }

        ssh_args
    }
}

#[async_trait]
impl CommandExecutor for SshExecutor {
    async fn execute_streaming(
        &self,
        program: &str,
        args: &[&str],
        host_prefix: &str,
        verbosity: Verbosity,
    ) -> Result<CommandHandle> {
        debug!(
            "Executing via SSH on {}: {} {}",
            self.host,
            program,
            args.join(" ")
        );

        let ssh_args = self.build_ssh_args();

        let mut cmd = Command::new("ssh");
        cmd.args(&ssh_args)
            .arg(program)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .map_err(|e| DeployError::ssh_connection_failed(&self.host, e.to_string()))?;

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let (stdout_tx, stdout_rx) = mpsc::channel(100);
        let (stderr_tx, stderr_rx) = mpsc::channel(100);

        // Spawn task to read stdout
        let stdout_prefix = format!("[{}]", host_prefix);
        let stdout_verbosity = verbosity;
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                match stdout_verbosity {
                    Verbosity::Progress => {
                        if line.contains("error") || line.contains("warning") || line.len() > 100 {
                            let _ = stdout_tx.send(format!("{} {}", stdout_prefix, line)).await;
                        }
                    }
                    Verbosity::Detailed => {
                        let _ = stdout_tx.send(format!("{} {}", stdout_prefix, line)).await;
                    }
                }
            }
        });

        // Spawn task to read stderr
        let stderr_prefix = format!("[{}]", host_prefix);
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                // Send stderr lines with host prefix but without automatic ERROR: prefix
                let _ = stderr_tx.send(format!("{} {}", stderr_prefix, line)).await;
            }
        });

        Ok(CommandHandle {
            child,
            stdout_receiver: stdout_rx,
            stderr_receiver: stderr_rx,
        })
    }

    async fn execute_and_wait(
        &self,
        program: &str,
        args: &[&str],
        timeout_duration: Option<Duration>,
    ) -> Result<CommandResult> {
        debug!(
            "Executing via SSH on {} and waiting: {} {}",
            self.host,
            program,
            args.join(" ")
        );

        let ssh_args = self.build_ssh_args();

        let mut cmd = Command::new("ssh");
        cmd.args(&ssh_args)
            .arg(program)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = cmd
            .spawn()
            .map_err(|e| DeployError::ssh_connection_failed(&self.host, e.to_string()))?;

        let execution = async {
            let output = child
                .wait_with_output()
                .await
                .map_err(|e| DeployError::ssh_connection_failed(&self.host, e.to_string()))?;

            let stdout_lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|s| s.to_string())
                .collect();

            let stderr_lines: Vec<String> = String::from_utf8_lossy(&output.stderr)
                .lines()
                .map(|s| s.to_string())
                .collect();

            if !output.status.success() {
                return Err(DeployError::command_failed(
                    format!("ssh {} {} {}", self.host, program, args.join(" ")),
                    output.status.code(),
                    stderr_lines.join("\n"),
                ));
            }

            Ok(CommandResult {
                exit_code: output.status.code(),
                stdout_lines,
                stderr_lines,
            })
        };

        if let Some(duration) = timeout_duration {
            timeout(duration, execution).await.map_err(|_| {
                DeployError::timeout(
                    format!("ssh {} {} {}", self.host, program, args.join(" ")),
                    duration.as_secs(),
                )
            })?
        } else {
            execution.await
        }
    }
}

/// Unified executor that can handle both local and SSH execution
#[derive(Debug, Clone)]
pub enum Executor {
    Local(LocalExecutor),
    Ssh(SshExecutor),
}

impl Executor {
    pub fn from_target(target: &ExecutionTarget) -> Self {
        match target {
            ExecutionTarget::Local => Self::Local(LocalExecutor::new()),
            ExecutionTarget::Ssh {
                host,
                username,
                port,
            } => {
                let mut executor = SshExecutor::new(host.clone());
                if let Some(username) = username {
                    executor = executor.with_username(username.clone());
                }
                if let Some(port) = port {
                    executor = executor.with_port(*port);
                }
                Self::Ssh(executor)
            }
        }
    }
}

#[async_trait]
impl CommandExecutor for Executor {
    async fn execute_streaming(
        &self,
        program: &str,
        args: &[&str],
        host_prefix: &str,
        verbosity: Verbosity,
    ) -> Result<CommandHandle> {
        match self {
            Self::Local(executor) => {
                executor
                    .execute_streaming(program, args, host_prefix, verbosity)
                    .await
            }
            Self::Ssh(executor) => {
                executor
                    .execute_streaming(program, args, host_prefix, verbosity)
                    .await
            }
        }
    }

    async fn execute_and_wait(
        &self,
        program: &str,
        args: &[&str],
        timeout_duration: Option<Duration>,
    ) -> Result<CommandResult> {
        match self {
            Self::Local(executor) => {
                executor
                    .execute_and_wait(program, args, timeout_duration)
                    .await
            }
            Self::Ssh(executor) => {
                executor
                    .execute_and_wait(program, args, timeout_duration)
                    .await
            }
        }
    }
}
