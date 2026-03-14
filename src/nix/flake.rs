use std::{collections::HashMap, sync::Arc};

use color_eyre::{Result, eyre::eyre};
use serde::Deserialize;

use crate::{
    executor::{Executor, RemoteHost},
    nix::store::{DrvPath, StorePath},
};

#[derive(Clone)]
pub struct NixFlake {
    pub flake_ref: String,
    pub executor: Arc<dyn Executor>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HostMeta {
    #[serde(default)]
    pub name: String,
    pub ips: HashMap<String, String>,
    #[allow(dead_code)]
    pub tags: Vec<String>,
}

pub struct TargetHost {
    meta: HostMeta,
    flake: NixFlake,
}

pub struct EvaluatedHost {
    meta: HostMeta,
    derivation_path: DrvPath,
}

pub struct BuiltHost {
    pub meta: HostMeta,
    pub store_path: StorePath,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalOutput {
    pub attr: String,
    pub attr_path: Vec<String>,
    pub drv_path: String,
    pub name: String,
    pub required_system_features: Vec<String>,
    pub system: String,
}

impl NixFlake {
    pub async fn evaluate_host_configs(&self) -> Result<Vec<HostMeta>> {
        let eval_output = self
            .executor
            .execute(&[
                "nix",
                "eval",
                "--json",
                format!("{}#nixosConfigurations", self.flake_ref).as_str(),
                "--apply",
                "builtins.mapAttrs (_: value: value.config.deploy)",
            ])
            .await?
            .into_result()?;

        let deploy_configs: HashMap<String, HostMeta> = serde_json::from_str(&eval_output)?;
        let deploy_configs = deploy_configs
            .into_iter()
            .map(|(name, mut meta)| {
                meta.name = name;
                meta
            })
            .collect::<Vec<_>>();

        Ok(deploy_configs)
    }
}

pub trait Evaluatable {
    type Output;
    async fn evaluate(&self) -> Result<Self::Output>;
}
impl TargetHost {
    pub fn new(meta: HostMeta, flake: NixFlake) -> Self {
        TargetHost { meta, flake }
    }
}
impl Evaluatable for TargetHost {
    type Output = EvaluatedHost;
    async fn evaluate(&self) -> Result<Self::Output> {
        let output = self
            .flake
            .executor
            .execute(&[
                "nix-eval-jobs",
                "--expr",
                "--workers",
                "8",
                format!(
                    r#"
                with builtins;
                let
                    flake = getFlake "{}";
                    lib = flake.inputs.nixpkgs.lib;
                in
                    flake.outputs.nixosConfigurations.{}.config.system.build.toplevel
                "#,
                    self.flake.flake_ref, self.meta.name,
                )
                .as_str(),
            ])
            .await?
            .into_result()?;

        if output.lines().count() != 1 {
            return Err(eyre!(
                "Expected single line output, got {} lines",
                output.lines().count()
            ));
        }

        Ok(EvaluatedHost {
            meta: self.meta.clone(),
            derivation_path: DrvPath {
                path: serde_json::from_str::<EvalOutput>(&output)
                    .unwrap()
                    .drv_path,
                host: self.flake.executor.clone(),
            },
        })
    }
}

impl Evaluatable for [TargetHost] {
    type Output = Vec<EvaluatedHost>;
    async fn evaluate(&self) -> Result<Self::Output> {
        let flake = self.first().unwrap().flake.clone();
        let output = flake
            .executor
            .execute(&[
                "nix-eval-jobs",
                "--expr",
                "--workers",
                "8",
                format!(
                    r#"
                      with builtins;
                      let
                          flake = getFlake "{}";
                          lib = flake.inputs.nixpkgs.lib;
                      in
                         lib.genAttrs [{}]
                         (host: flake.outputs.nixosConfigurations."${{host}}".config.system.build.toplevel)
                      "#,
                    flake.flake_ref, self.iter().map(|s| format!("\"{}\"", s.meta.name)).collect::<Vec<_>>().join(" "),
                )
                .as_str(),
            ])
            .await?
            .into_result()?;

        if output.lines().count() != self.len() {
            return Err(eyre!(
                "Expected {} line output, got {} lines",
                self.len(),
                output.lines().count()
            ));
        }

        let hosts = output
            .lines()
            .map(|x| {
                let eval_output = serde_json::from_str::<EvalOutput>(x).unwrap();
                let host = self
                    .iter()
                    .find(|x| x.meta.name == eval_output.attr)
                    .unwrap();
                EvaluatedHost {
                    meta: host.meta.clone(),
                    derivation_path: DrvPath {
                        path: eval_output.drv_path,
                        host: flake.executor.clone(),
                    },
                }
            })
            .collect();

        Ok(hosts)
    }
}

impl EvaluatedHost {
    pub async fn realise(self) -> Result<BuiltHost> {
        let path = self.derivation_path.realise().await?;

        Ok(BuiltHost {
            meta: self.meta,
            store_path: path,
        })
    }
}

impl HostMeta {
    pub async fn connect(&self) -> std::result::Result<RemoteHost, openssh::Error> {
        RemoteHost::connect(self.ips.get("primary").unwrap(), self.name.clone()).await
    }
}
