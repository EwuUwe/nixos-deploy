#![warn(clippy::pedantic)]
#![allow(clippy::assigning_clones)]

use std::sync::Arc;

use crate::{
    cli::Commands,
    executor::LocalHost,
    host::TargetHost,
    nix::flake::{Evaluatable, NixFlake},
};
use clap::Parser;
use color_eyre::{Result, eyre::eyre};
use tokio::task::JoinSet;

mod cli;
mod executor;
mod host;
mod nix;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let args = cli::Cli::parse();

    let localhost = Arc::new(LocalHost);
    let nixflake = NixFlake {
        flake_ref: args.flake,
        executor: localhost,
    };

    match args.command {
        Commands::Eval { ref common }
        | Commands::Apply { ref common }
        | Commands::Push { ref common }
        | Commands::Build { ref common } => {
            let host_metas = nixflake.evaluate_host_configs().await?;

            let missing_hosts: Vec<&str> = common
                .hosts
                .iter()
                .filter(|s| !host_metas.contains_key(*s))
                .map(std::string::String::as_str)
                .collect();

            if !missing_hosts.is_empty() {
                return Err(eyre!(
                    "Specified hosts not found in flake: {}",
                    missing_hosts.join(",")
                ));
            }

            let target_metas = common.hosts.iter().filter_map(|s| host_metas.get(s));

            let target_hosts: Vec<_> = target_metas
                .map(|x| TargetHost::new(x.clone(), nixflake.clone()))
                .collect();

            let evaluated_targets = target_hosts.evaluate().await?;

            if matches!(args.command, Commands::Eval { .. }) {
                return Ok(());
            }

            let mut threads = JoinSet::<Result<()>>::new();
            for evaluated_target in evaluated_targets {
                let command = args.command.clone();
                threads.spawn(async move {
                    println!("Building {}", evaluated_target.meta.name);
                    let res = evaluated_target.realise().await?;
                    println!("Built: {}", res.store_path.path);

                    if matches!(command, Commands::Build { .. }) {
                        return Ok(());
                    }

                    let executor = Arc::new(res.meta.connect().await?);
                    println!("Copying to {executor}");
                    res.store_path.copy_to(executor).await?;
                    println!("Copied");

                    if matches!(command, Commands::Push { .. }) {
                        return Ok(());
                    }

                    todo!();
                });
            }

            while let Some(res) = threads.join_next().await {
                res??;
            }

            return Ok(());
        }
        _ => todo!(),
    }
}
