use std::collections::HashMap;

use color_eyre::Result;
use serde::Deserialize;

use crate::{
    executor::RemoteHost,
    nix::{
        flake::NixFlake,
        store::{DrvPath, StorePath},
    },
};

#[derive(Debug, Deserialize, Clone)]
pub struct HostMeta {
    #[serde(default)]
    pub name: String,
    pub ips: HashMap<String, String>,
    #[allow(dead_code)]
    pub tags: Vec<String>,
}

impl HostMeta {
    pub async fn connect(&self) -> std::result::Result<RemoteHost, openssh::Error> {
        RemoteHost::connect(self.ips.get("primary").unwrap(), self.name.clone()).await
    }
}

pub struct TargetHost {
    pub meta: HostMeta,
    pub flake: NixFlake,
}

impl TargetHost {
    pub const fn new(meta: HostMeta, flake: NixFlake) -> Self {
        Self { meta, flake }
    }
}

pub struct EvaluatedHost {
    pub meta: HostMeta,
    pub derivation_path: DrvPath,
}

impl EvaluatedHost {
    pub async fn realise(self, target: String) -> Result<BuiltHost> {
        let path = self.derivation_path.realise(target).await?;

        Ok(BuiltHost {
            meta: self.meta,
            store_path: path,
        })
    }
}

pub struct BuiltHost {
    pub meta: HostMeta,
    pub store_path: StorePath,
}
