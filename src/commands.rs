use color_eyre::Result;

use crate::{
    executor::Executor,
    nix::flake::NixFlake,
    pipeline::resolve_targets,
};

pub async fn show(flake: &NixFlake) -> Result<()> {
    let hosts = flake.evaluate_host_configs().await?;
    println!("{hosts:#?}");
    Ok(())
}

pub async fn exec(flake: &NixFlake, hosts: &[String], command: &str) -> Result<()> {
    let targets = resolve_targets(flake, hosts).await?;

    for target in &targets {
        let connection = target.meta.connect().await?;
        let output = connection.execute(&[command]).await?.into_result()?;
        println!("{output}");
    }

    Ok(())
}
