use openssh::{KnownHosts, Session};
use std::{
    fmt::{self, Debug, Display},
    process::ExitStatus,
};
use tokio::process::Command;

use async_trait::async_trait;

#[derive(Debug)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub status: ExitStatus,
}

#[derive(Debug)]
pub struct CommandError {
    pub stderr: String,
    pub status: ExitStatus,
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Command failed with status {}: {}",
            self.status, self.stderr
        )
    }
}

impl std::error::Error for CommandError {}

impl CommandOutput {
    pub fn into_result(self) -> Result<String, CommandError> {
        if self.status.success() {
            Ok(self.stdout)
        } else {
            Err(CommandError {
                stderr: self.stderr,
                status: self.status,
            })
        }
    }
}

#[async_trait]
pub trait Executor: Send + Sync + Display + Debug {
    async fn execute(&self, command: &[&str]) -> color_eyre::Result<CommandOutput>;
    fn store_uri(&self) -> String;
}

#[derive(Debug)]
pub struct RemoteHost {
    session: Session,
    target: String,
    name: String,
}

impl RemoteHost {
    pub async fn connect(destination: &str, name: String) -> Result<Self, openssh::Error> {
        let session = Session::connect_mux(destination, KnownHosts::Strict).await?;

        Ok(Self {
            session,
            target: destination.to_string(),
            name,
        })
    }
}

impl Display for RemoteHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

#[async_trait]
impl Executor for RemoteHost {
    async fn execute(&self, command: &[&str]) -> color_eyre::Result<CommandOutput> {
        let output = self
            .session
            .command(command[0])
            .args(&command[1..])
            .output()
            .await?;

        Ok(CommandOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            status: output.status,
        })
    }
    fn store_uri(&self) -> String {
        format!("ssh://{}", self.target)
    }
}

#[derive(Debug)]
pub struct LocalHost;

impl Display for LocalHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "localhost")
    }
}

#[async_trait]
impl Executor for LocalHost {
    async fn execute(&self, command: &[&str]) -> color_eyre::Result<CommandOutput> {
        let output = Command::new(command[0])
            .args(&command[1..])
            .output()
            .await?;

        Ok(CommandOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            status: output.status,
        })
    }

    fn store_uri(&self) -> String {
        "auto".to_string()
    }
}
