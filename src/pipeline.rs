use std::{collections::HashSet, sync::Arc};

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

    let mut matched_keys = HashSet::new();
    let mut missing_patterns = Vec::new();

    for pattern in hosts {
        let wm = wildmatch::WildMatch::new(pattern);
        let mut matches = host_metas.keys().filter(|k| wm.matches(k)).peekable();

        if matches.peek().is_none() {
            missing_patterns.push(pattern.as_str());
        } else {
            matched_keys.extend(matches);
        }
    }

    if !missing_patterns.is_empty() {
        return Err(eyre!(
            "Specified host patterns matched no hosts in flake: {}",
            missing_patterns.join(", ")
        ));
    }

    Ok(matched_keys
        .into_iter()
        .map(|key| TargetHost::new(host_metas[key].clone(), flake.clone()))
        .collect())
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
    let target_name = evaluated_host.meta.name.clone();
    let built = evaluated_host.realise(target_name).await?;
    println!("Built: {}", built.store_path.path);

    if stage < Stage::Push {
        return Ok(());
    }

    let executor = Arc::new(built.meta.connect().await?);
    println!("Copying to {executor}");
    let target_store_path = built.store_path.copy_to(executor).await?;
    println!("Copied");

    if stage < Stage::Apply {
        return Ok(());
    }

    target_store_path.activate().await?;

    Ok(())
}
