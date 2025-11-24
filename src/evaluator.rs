use std::{collections::HashMap, path::PathBuf, process::Stdio};

use async_stream::try_stream;
use color_eyre::{
    Result,
    eyre::{Context, ContextCompat, eyre},
};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::mpsc::UnboundedSender,
};
use tokio_stream::Stream;

use crate::{
    execution::{CommandExecutor, Executor, SshExecutor},
    types::ConfigurationStatus,
};

pub struct Flake {
    pub path: PathBuf,
    pub nixos_configurations: HashMap<String, NixosConfiguration>,
}

#[derive(Debug, Clone)]
pub struct NixosConfiguration {
    pub name: String,
    pub metadata: Metadata,
    pub executor: Executor,
    pub drv_path: Option<String>,
    pub out_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Metadata {
    pub system: String,
    pub ips: serde_json::Value,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct NixEvalJobOutput {
    #[serde(rename = "drvPath")]
    pub drv_path: Option<String>,
    pub outputs: Option<NixOutputs>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub system: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct NixOutputs {
    pub out: String,
}

impl Flake {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            nixos_configurations: HashMap::new(),
        }
    }

    pub async fn evaluate_configuration_deployment_options(&mut self) -> Result<()> {
        let output = Command::new("nix")
            .arg("eval")
            .arg("--json")
            .arg(format!(
                "path:{}#nixosConfigurations",
                self.path.to_str().unwrap()
            ))
            .arg("--apply")
            .arg("builtins.mapAttrs (_: value: value.config.deploy)")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to execute nix eval command")?;

        if !output.status.success() {
            return Err(eyre!(
                "nix eval failed: {}",
                String::from_utf8_lossy(&output.stderr),
            ));
        }

        let config_names: HashMap<String, Metadata> =
            serde_json::from_slice::<HashMap<String, Metadata>>(&output.stdout)
                .context("Failed to parse configuration names JSON")?;

        self.nixos_configurations = config_names
            .into_iter()
            .map(|(name, meta)| {
                let primary_ip = meta.ips["primary"].to_string();
                (
                    name.clone(),
                    NixosConfiguration {
                        name,
                        metadata: meta,
                        executor: Executor::Ssh(SshExecutor::new(primary_ip)),
                        drv_path: None,
                        out_path: None,
                    },
                )
            })
            .collect();

        Ok(())
    }

    pub async fn evaluate_configurations(
        &mut self,
    ) -> Result<impl Stream<Item = Result<NixosConfiguration>> + Send> {
        let mut child = Command::new("nix-eval-jobs")
            .arg("--workers")
            .arg("12")
            .arg("--expr")
            .arg(format!(
                r#"
                with builtins;
                let flake = getFlake "path:{}";
                in mapAttrs
                    (name: config: config.config.system.build.toplevel)
                    flake.outputs.nixosConfigurations
                "#,
                self.path.to_string_lossy()
            ))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to start nix-eval-jobs")?;

        let stdout = child.stdout.take().context("Failed to get stdout")?;
        let stderr = child.stderr.take().context("Failed to get stderr")?;

        let stdout_reader = BufReader::new(stdout);
        let stderr_reader = BufReader::new(stderr);

        let mut stdout_lines = stdout_reader.lines();
        let mut stderr_lines = stderr_reader.lines();

        Ok(try_stream! {
            loop {
                tokio::select! {
                    line = stdout_lines.next_line() => {
                        match line.unwrap() {
                            Some(line) => {
                                if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&line) {
                                    if let Some(attr) = json_value.get("attr") {
                                        if let Some(attr_str) = attr.as_str() {
                                            if let Ok(output) = serde_json::from_value::<NixEvalJobOutput>(json_value.clone()) {
                                                let config = self.nixos_configurations.get_mut(attr_str).unwrap();
                                                config.drv_path = output.drv_path.clone();
                                                config.out_path = output.outputs.as_ref().map(|o| o.out.clone());

                                                yield (config.clone());
                                            }
                                        }
                                    }
                                }
                            },
                            None => break,
                        }
                    }
                    line = stderr_lines.next_line() => {
                        if let Ok(Some(_)) = line {
                            // Handle stderr if needed
                        }
                    }
                }
            }
        })
    }
}

impl NixosConfiguration {
    pub async fn build(&self) -> Result<()> {
        let drv_path = self.drv_path.clone().unwrap();
        let child = Command::new("nix-build")
            .arg(drv_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to start nix build")?;

        let output = child
            .wait_with_output()
            .await
            .context("Failed to wait on nix build")?;

        if !output.status.success() {
            return Err(eyre!(
                "nix build failed: {}",
                String::from_utf8_lossy(&output.stderr),
            ));
        }

        Ok(())
    }

    pub async fn activate(&self, handle: &UnboundedSender<ConfigurationStatus>) -> Result<()> {
        let out_path = self.out_path.clone().unwrap();
        let res = self
            .executor
            .execute_and_wait(
                &format!("{out_path}/bin/switch-to-configuration"),
                &["switch"],
                Some(std::time::Duration::from_secs(600)),
            )
            .await
            .context("Failed to execute activation command")?;

        for line in &res.stderr_lines {
            handle
                .send(ConfigurationStatus::Error {
                    error: line.clone(),
                    error_lines: vec![line.clone()],
                })
                .unwrap();
        }

        Ok(())
    }
}
