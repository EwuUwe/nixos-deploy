use clap::Parser;
use cli::{Cli, Commands};
use display::StatusManager;
use std::pin::pin;
use std::{collections::HashMap, path::PathBuf};
use tokio::task::JoinSet;
use tokio_stream::StreamExt;

mod cli;
mod display;
mod evaluator;
mod executor;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Some(Commands::Build { flake }) => {
            let default_flake = PathBuf::from(".");
            let flake_ref = flake.as_ref().unwrap_or(&default_flake);

            let status_manager = StatusManager::new();

            let mut flake = evaluator::Flake::new(flake_ref.clone());
            flake.evaluate_configuration_deployment_options().await?;

            let config_names: Vec<_> = flake.nixos_configurations.keys().cloned().collect();

            let handles: HashMap<String, display::StatusHandle> = config_names
                .iter()
                .map(|config_name| {
                    let handle = status_manager.add_configuration(config_name.as_str());
                    handle
                        .tx
                        .send(display::ConfigurationStatus::Started(config_name.clone()))
                        .unwrap();
                    handle
                        .tx
                        .send(display::ConfigurationStatus::Evaluating)
                        .unwrap();
                    (config_name.clone(), handle)
                })
                .collect();

            let mut build_jobs = JoinSet::new();

            let eval_stream = flake.evaluate_configurations().await?;
            let mut eval_stream = pin!(eval_stream);

            while let Some(config) = eval_stream.next().await {
                let config = config?;
                let config_name = config.name.clone();
                let handle = handles.get(&config_name).unwrap();

                handle
                    .tx
                    .send(display::ConfigurationStatus::Evaluated {
                        drvPath: config.drv_path.clone().unwrap_or_default(),
                    })
                    .unwrap();

                let tx = handle.tx.clone();
                build_jobs.spawn(async move {
                    tx.send(display::ConfigurationStatus::Building).unwrap();
                    if let Err(_) = config.build().await {
                        tx.send(display::ConfigurationStatus::Error {
                            error: "Build failed".to_string(),
                        })
                        .unwrap();
                    } else {
                        tx.send(display::ConfigurationStatus::Built {
                            result: config.out_path.clone().unwrap_or_default(),
                        })
                        .unwrap();
                    }

                    let _ = config.activate(&tx).await;
                });
            }

            build_jobs.join_all().await;

            //status_manager.shutdown();
        }
        None => {
            println!("Use --help for usage information");
        }
    }

    Ok(())
}
