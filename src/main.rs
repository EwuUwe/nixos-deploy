#![warn(clippy::pedantic)]

use std::sync::Arc;

use crate::{
    executor::LocalHost,
    nix::flake::{Evaluatable, NixFlake, TargetHost},
};
use color_eyre::Result;
use tokio::task::JoinSet;

mod executor;
mod nix;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let localhost = Arc::new(LocalHost);
    let nixflake = NixFlake {
        flake_ref: "path:/etc/nixos".to_string(),
        executor: localhost,
    };

    println!("Evaluating flake host configs");
    let host_metas = nixflake.evaluate_host_configs().await?;

    let hosts = ["monitoring", "auth"];

    let targets: Vec<TargetHost> = host_metas
        .into_iter()
        .filter(|x| hosts.contains(&x.name.as_str()))
        .map(|x| TargetHost::new(x, nixflake.clone()))
        .collect();

    println!("Evaluating selected configs");
    let evaluated = targets.evaluate().await?;

    let mut threads = JoinSet::<color_eyre::Result<()>>::new();
    for evaluated_host in evaluated {
        threads.spawn(async move {
            //println!("Building {}", evaluated_host);
            let res = evaluated_host.realise().await?;
            println!("Built: {}", res.store_path.path);
            let executor = Arc::new(res.meta.connect().await?);
            println!("Copying to {executor}");
            res.store_path.copy_to(executor).await?;
            println!("Copied");

            Ok(())
        });
    }

    while let Some(res) = threads.join_next().await {
        res??;
    }

    Ok(())
}
