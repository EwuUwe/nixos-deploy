use std::sync::Arc;

use color_eyre::{Result, eyre::eyre};
use tokio::task::JoinSet;

use crate::{
    host::{EvaluatedHost, TargetHost},
    nix::flake::{Evaluatable, NixFlake},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    Eval,
    Build,
    Push,
    Apply,
}

pub async fn resolve_targets(flake: &NixFlake, hosts: &[String]) -> Result<Vec<TargetHost>> {
    let host_metas = flake.evaluate_host_configs().await?;

    let missing_hosts: Vec<&str> = hosts
        .iter()
        .filter(|s| !host_metas.contains_key(*s))
        .map(String::as_str)
        .collect();

    if !missing_hosts.is_empty() {
        return Err(eyre!(
            "Specified hosts not found in flake: {}",
            missing_hosts.join(", ")
        ));
    }

    let targets = hosts
        .iter()
        .filter_map(|s| host_metas.get(s))
        .map(|meta| TargetHost::new(meta.clone(), flake.clone()))
        .collect();

    Ok(targets)
}

pub async fn deploy(stage: Stage, targets: Vec<TargetHost>) -> Result<()> {
    println!(
        "Evaluating hosts: {}",
        targets
            .iter()
            .map(|x| x.meta.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let evaluated_targets = targets.evaluate().await?;

    if stage == Stage::Eval {
        return Ok(());
    }

    let mut tasks = JoinSet::<Result<()>>::new();
    for target in evaluated_targets {
        tasks.spawn(process_host(stage, target));
    }

    let mut failures = 0u32;
    while let Some(result) = tasks.join_next().await {
        match result {
            Err(join_err) => {
                eprintln!("Host task panicked: {join_err}");
                failures += 1;
            }
            Ok(Err(err)) => {
                eprintln!("Host failed: {err:?}");
                failures += 1;
            }
            Ok(Ok(())) => {}
        }
    }

    if failures > 0 {
        return Err(eyre!("{failures} host(s) failed"));
    }

    Ok(())
}

async fn process_host(stage: Stage, evaluated_host: EvaluatedHost) -> Result<()> {
    println!("Building {}", evaluated_host.meta.name);
    let built = evaluated_host.realise().await?;
    println!("Built: {}", built.store_path.path);

    if stage < Stage::Push {
        return Ok(());
    }

    let executor = Arc::new(built.meta.connect().await?);
    println!("Copying to {executor}");
    built.store_path.copy_to(executor).await?;
    println!("Copied");

    if stage < Stage::Apply {
        return Ok(());
    }

    todo!();
}
