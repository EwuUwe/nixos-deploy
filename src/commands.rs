use color_eyre::Result;
use tokio::task::JoinSet;

use crate::{
    executor::{ExecutionContext, Executor},
    nix::flake::NixFlake,
    pipeline::resolve_targets,
};

pub async fn show(flake: &NixFlake) -> Result<()> {
    let hosts = flake.evaluate_host_configs().await?;
    println!("{hosts:#?}");
    Ok(())
}

pub async fn exec(flake: &NixFlake, hosts: &[String], command: &[String]) -> Result<()> {
    let targets = resolve_targets(flake, hosts).await?;

    let mut tasks = JoinSet::<Result<()>>::new();
    for target in targets {
        let command = command.to_vec();
        tasks.spawn(async move {
            let connection = target.meta.connect().await?;
            let command: Vec<&str> = command.iter().map(String::as_str).collect();
            let output = connection
                .execute(&command, ExecutionContext::This)
                .await?
                .into_result()?;
            println!("{output}");

            Ok(())
        });
    }

    while let Some(result) = tasks.join_next().await {
        result??;
    }

    Ok(())
}
