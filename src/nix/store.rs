use crate::executor::Executor;
use color_eyre::Result;
use std::sync::Arc;

/// Reference to an existing /nix/store-Path on a specific host
pub struct StorePath {
    pub path: String,
    pub host: Arc<dyn Executor>,
}

/// Reference to an existing /nix/store/*.drv-Derivation-Path on a specific host
pub struct DrvPath {
    pub path: String,
    pub host: Arc<dyn Executor>,
}

impl DrvPath {
    pub async fn realise(self) -> Result<StorePath> {
        let output = self
            .host
            .execute(&["nix-store", "--realise", &self.path])
            .await?
            .into_result()?;

        Ok(StorePath {
            path: output.lines().next().unwrap().to_string(),
            host: self.host,
        })
    }
}

impl StorePath {
    pub async fn copy_to(&self, target: Arc<dyn Executor>) -> Result<StorePath> {
        let executor = crate::executor::LocalHost;
        let _output = executor
            .execute(&[
                "nix",
                "copy",
                "--from",
                self.host.store_uri(),
                "--to",
                target.store_uri(),
                self.path.as_str(),
            ])
            .await?
            .into_result()?;

        Ok(StorePath {
            path: self.path.clone(),
            host: target,
        })
    }
}
