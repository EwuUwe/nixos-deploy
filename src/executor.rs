use openssh::{KnownHosts, Session};
use std::{
    fmt::{self, Debug, Display},
    process::ExitStatus,
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, BufReader},
    process::Command,
};

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

pub enum ExecutionContext {
    This,
    Cross(String),
}

impl ExecutionContext {
    fn fmt(&self, this: &str) -> String {
        match self {
            ExecutionContext::This => format!("[{}]", this),
            ExecutionContext::Cross(remote) => {
                format!("[{} => {}]", this, remote)
            }
        }
    }
}

#[async_trait]
pub trait Executor: Send + Sync + Display + Debug {
    async fn execute(
        &self,
        command: &[&str],
        context: ExecutionContext,
    ) -> color_eyre::Result<CommandOutput>;
    fn store_uri(&self) -> &str;
}

#[derive(Debug)]
pub struct RemoteHost {
    session: Session,
    store_uri: String,
    name: String,
}

impl RemoteHost {
    pub async fn connect(destination: &str, name: String) -> Result<Self, openssh::Error> {
        let session = Session::connect_mux(destination, KnownHosts::Strict).await?;

        Ok(Self {
            session,
            store_uri: format!("ssh://{destination}"),
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
    async fn execute(
        &self,
        command: &[&str],
        context: ExecutionContext,
    ) -> color_eyre::Result<CommandOutput> {
        let mut child = self
            .session
            .command(command[0])
            .args(&command[1..])
            .stdout(openssh::Stdio::piped())
            .stderr(openssh::Stdio::piped())
            .spawn()
            .await?;

        let prefix = context.fmt(&self.name);
        let (stdout, stderr) = stream_output(
            BufReader::new(child.stdout().take().unwrap()),
            BufReader::new(child.stderr().take().unwrap()),
            prefix,
        )
        .await?;

        let status = child.wait().await?;

        Ok(CommandOutput {
            stdout,
            stderr,
            status,
        })
    }
    fn store_uri(&self) -> &str {
        &self.store_uri
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
    async fn execute(
        &self,
        command: &[&str],
        context: ExecutionContext,
    ) -> color_eyre::Result<CommandOutput> {
        let mut child = Command::new(command[0])
            .args(&command[1..])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        let prefix = context.fmt(&self.to_string());
        let (stdout, stderr) = stream_output(
            BufReader::new(child.stdout.take().unwrap()),
            BufReader::new(child.stderr.take().unwrap()),
            prefix,
        )
        .await?;

        let status = child.wait().await?;
        Ok(CommandOutput {
            stdout,
            stderr,
            status,
        })
    }

    fn store_uri(&self) -> &'static str {
        "auto"
    }
}

const PREFIX_WIDTH: usize = 20;

async fn stream_output<O, E>(
    stdout_reader: O,
    stderr_reader: E,
    prefix: String,
) -> color_eyre::Result<(String, String)>
where
    O: AsyncBufRead + Unpin,
    E: AsyncBufRead + Unpin,
{
    let mut stdout_lines = stdout_reader.lines();
    let mut stderr_lines = stderr_reader.lines();
    let mut out_buf = String::new();
    let mut err_buf = String::new();

    let mut stdout_done = false;
    let mut stderr_done = false;

    while !stdout_done || !stderr_done {
        tokio::select! {
            line = stdout_lines.next_line(), if !stdout_done => {
                match line? {
                    Some(line) => {
                        println!("{:<width$} | {}", prefix, line, width = PREFIX_WIDTH);
                        out_buf.push_str(&line);
                        out_buf.push('\n');
                    }
                    None => stdout_done = true,
                }
            }
            line = stderr_lines.next_line(), if !stderr_done => {
                match line? {
                    Some(line) => {
                        eprintln!("{:<width$} | {}", prefix, line, width = PREFIX_WIDTH);
                        err_buf.push_str(&line);
                        err_buf.push('\n');
                    }
                    None => stderr_done = true,
                }
            }
        }
    }

    Ok((out_buf, err_buf))
}
