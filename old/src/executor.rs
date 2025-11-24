use std::process::Stdio;

use async_trait::async_trait;
use color_eyre::Result;
use tokio::{
    io::BufReader,
    process::{Child, ChildStderr, ChildStdout, Command},
};

#[derive(Debug, Clone)]
pub enum Executor {
    Local(LocalExecutor),
    Ssh(SshExecutor),
}

pub struct CommandResult {
    pub stdout: BufReader<ChildStdout>,
    pub stderr: BufReader<ChildStderr>,
    pub child: Child,
}

#[async_trait]
pub trait CommandExecutor {
    async fn execute(&self, command: &str, args: &[&str]) -> Result<CommandResult>;
}

#[derive(Debug, Clone)]
pub struct LocalExecutor;
impl LocalExecutor {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl CommandExecutor for LocalExecutor {
    async fn execute(&self, program: &str, args: &[&str]) -> Result<CommandResult> {
        let mut cmd = Command::new(program);
        cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn()?;

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        Ok(CommandResult {
            stdout: BufReader::new(stdout),
            stderr: BufReader::new(stderr),
            child,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SshExecutor {
    host: String,
    username: Option<String>,
    port: Option<u16>,
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
}

#[async_trait]
impl CommandExecutor for SshExecutor {
    async fn execute(&self, program: &str, args: &[&str]) -> Result<CommandResult> {
        let mut ssh_args: Vec<String> = Vec::new();
        if let Some(port) = self.port {
            ssh_args.push("-p".to_string());
            ssh_args.push(port.to_string());
        }

        if let Some(username) = &self.username {
            ssh_args.push(format!("{}@{}", username, self.host));
        } else {
            ssh_args.push(self.host.clone());
        }

        let mut child = Command::new("ssh")
            .args(ssh_args)
            .arg(program)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        Ok(CommandResult {
            stdout: BufReader::new(stdout),
            stderr: BufReader::new(stderr),
            child,
        })
    }
}

#[async_trait]
impl CommandExecutor for Executor {
    async fn execute(&self, command: &str, args: &[&str]) -> Result<CommandResult> {
        match self {
            Executor::Local(executor) => executor.execute(command, args).await,
            Executor::Ssh(executor) => executor.execute(command, args).await,
        }
    }
}
