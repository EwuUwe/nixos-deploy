use std::{collections::HashMap, sync::Arc};

use color_eyre::{Result, eyre::eyre};

use crate::{
    executor::{ExecutionContext, Executor},
    host::{EvaluatedHost, HostMeta, TargetHost},
    nix::{eval::EvalOutput, store::DrvPath},
};

#[derive(Clone)]
pub struct NixFlake {
    pub flake_ref: String,
    pub executor: Arc<dyn Executor>,
}

impl NixFlake {
    pub async fn evaluate_host_configs(&self) -> Result<HashMap<String, HostMeta>> {
        let eval_output = self
            .executor
            .execute(
                &[
                    "nix",
                    "eval",
                    "--json",
                    format!("{}#nixosConfigurations", self.flake_ref).as_str(),
                    "--apply",
                    "builtins.mapAttrs (_: value: value.config.deploy)",
                ],
                ExecutionContext::This,
            )
            .await?
            .into_result()?;

        let host_metas = serde_json::from_str::<HashMap<String, HostMeta>>(&eval_output)?
            .into_iter()
            .map(|(name, mut meta)| {
                meta.name = name.clone();
                (name, meta)
            })
            .collect();

        Ok(host_metas)
    }
}

pub trait Evaluatable {
    type Output;
    async fn evaluate(&self) -> Result<Self::Output>;
}

impl Evaluatable for TargetHost {
    type Output = EvaluatedHost;
    async fn evaluate(&self) -> Result<Self::Output> {
        let output = self
            .flake
            .executor
            .execute(
                &[
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
                ],
                ExecutionContext::This,
            )
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
                    flake.flake_ref,
                    self.iter()
                        .map(|s| format!("\"{}\"", s.meta.name))
                        .collect::<Vec<_>>()
                        .join(" "),
                )
                .as_str(),
            ],
            ExecutionContext::This,
        )
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
