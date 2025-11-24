use clap::Parser;
use owo_colors::OwoColorize;
use std::{collections::HashMap, path::PathBuf, pin::pin};
use tokio::task::JoinSet;
use tokio_stream::StreamExt;
use tracing::{error, info};
use tracing_subscriber;

mod cli;
mod display;
mod errors;
mod execution;
mod operations;
mod types;

// Keep old modules for now (will be removed after refactoring is complete)
mod evaluator;
mod executor;

use cli::{Cli, Commands};
use display::StatusManager;
use errors::Result;
use operations::{Builder, Deployer, Evaluator};
use types::{ConfigurationStatus, DeploymentOptions, NixosConfiguration, Verbosity};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing based on verbosity
    let deployment_options = cli.parse_deployment_options().unwrap_or_default();

    match deployment_options.verbosity {
        Verbosity::Progress => {
            // For progress mode, show INFO and ERROR but not WARN/DEBUG to avoid interfering with progress bars
            tracing_subscriber::fmt()
                .with_target(false)
                .with_level(false)
                .without_time()
                .with_max_level(tracing::Level::INFO)
                .init();
        }
        Verbosity::Detailed => {
            // For detailed mode, show all logs with levels but without timestamps/modules
            tracing_subscriber::fmt()
                .with_target(false)
                .with_level(true)
                .without_time()
                .with_max_level(tracing::Level::DEBUG)
                .init();
        }
    }

    match &cli.command {
        Some(Commands::Build { flake, configs, .. }) => {
            let default_flake = PathBuf::from(".");
            let flake_ref = flake.as_ref().unwrap_or(&default_flake);

            let deployment_options = cli.parse_deployment_options().unwrap_or_default();

            run_deployment(flake_ref.clone(), deployment_options, configs.clone()).await
        }
        None => {
            println!("Use --help for usage information");
            Ok(())
        }
    }
}

async fn run_deployment(
    flake_path: PathBuf,
    options: DeploymentOptions,
    target_configs: Vec<String>,
) -> Result<()> {
    // Show starting message
    info!("Starting deployment of flake at {:?}", flake_path);

    let status_manager = StatusManager::new(options.verbosity);

    // Step 1: Evaluate deployment metadata
    let evaluator = Evaluator::new(
        flake_path.clone(),
        options
            .evaluation_host_override
            .as_ref()
            .unwrap_or(&types::ExecutionTarget::Local),
    );

    let metadata = evaluator.evaluate_deployment_metadata().await?;

    if metadata.is_empty() {
        eprintln!(
            "{}: No NixOS configurations found in flake",
            "ERROR".bright_red()
        );
        return Ok(());
    }

    // Show configuration count
    info!("Found {} configurations", metadata.len());

    // Step 2: Create configuration objects and apply overrides
    let mut configurations: HashMap<String, NixosConfiguration> = HashMap::new();
    for (name, meta) in metadata {
        if !target_configs.is_empty() && !target_configs.contains(&name) {
            continue;
        }

        match NixosConfiguration::new(name.clone(), meta) {
            Ok(mut config) => {
                config.deployment_config.apply_overrides(&options);
                configurations.insert(name, config);
            }
            Err(e) => {
                eprintln!("Failed to create configuration '{}': {}", name, e);
                continue;
            }
        }
    }

    if configurations.is_empty() {
        eprintln!("No valid configurations to deploy");
        return Ok(());
    }

    // Step 3: Create status handles
    let handles: HashMap<String, display::StatusHandle> = configurations
        .keys()
        .map(|config_name| {
            let handle = status_manager.add_configuration(config_name);
            let _ = handle
                .tx
                .send(ConfigurationStatus::Started(config_name.clone()));
            let _ = handle.tx.send(ConfigurationStatus::Evaluating);
            (config_name.clone(), handle)
        })
        .collect();

    // Step 4: Evaluate configurations and start deployment immediately
    let eval_stream = evaluator.evaluate_configurations().await?;
    let mut eval_stream = pin!(eval_stream);

    let mut deployment_tasks = JoinSet::new();

    while let Some(eval_result) = eval_stream.next().await {
        match eval_result {
            Ok(output) => {
                if let Some(attr) = &output.attr {
                    if let Some(config) = configurations.get(attr) {
                        let updated_config = config.clone().with_paths(
                            output.drv_path.clone(),
                            output.outputs.as_ref().map(|o| o.out.clone()),
                        );

                        if let Some(handle) = handles.get(attr) {
                            let _ = handle.tx.send(ConfigurationStatus::Evaluated {
                                drv_path: output.drv_path.unwrap_or_default(),
                            });

                            // Start deployment immediately after evaluation
                            let config_clone = updated_config.clone();
                            let handle_clone = handle.tx.clone();
                            let verbosity = options.verbosity;

                            deployment_tasks.spawn(async move {
                                deploy_single_configuration(config_clone, handle_clone, verbosity)
                                    .await
                            });
                        }
                    }
                }
            }
            Err(e) => {
                // Only log evaluation errors in detailed mode, progress bars will show them
                if matches!(options.verbosity, Verbosity::Detailed) {
                    error!("Evaluation error: {}", e);
                }
            }
        }
    }

    // Step 5: Wait for all deployments to complete

    // Step 5: Wait for all deployments to complete
    let mut results = Vec::new();
    while let Some(result) = deployment_tasks.join_next().await {
        match result {
            Ok(deploy_result) => results.push(deploy_result),
            Err(e) => {
                // Only log task failures in detailed mode (deployment errors are already shown)
                if matches!(options.verbosity, Verbosity::Detailed) {
                    eprintln!("ERROR: Deployment task failed: {}", e);
                }
                results.push(Err(errors::DeployError::Other(e.to_string())));
            }
        }
    }

    // Shutdown status manager to show buffered errors if in Progress mode
    status_manager.shutdown();

    // Process results - just show a simple summary
    let (successes, failures): (Vec<_>, Vec<_>) = results.into_iter().partition(Result::is_ok);

    // Show final summary
    match options.verbosity {
        Verbosity::Progress => {
            if failures.is_empty() {
                println!(
                    "\n✓ All {} configurations deployed successfully",
                    successes.len()
                );
            } else {
                println!(
                    "\n✗ Deployment completed: {} successes, {} failures",
                    successes.len(),
                    failures.len()
                );
                println!("  (Errors shown above for each failed configuration)");
            }
        }
        Verbosity::Detailed => {
            println!(
                "INFO: Deployment completed: {} successes, {} failures",
                successes.len(),
                failures.len()
            );
        }
    }

    // Return error code if there were failures (for scripts/CI)
    // but don't print redundant error message since we already showed detailed summary above
    if !failures.is_empty() {
        std::process::exit(1);
    }

    Ok(())
}

async fn deploy_single_configuration(
    config: NixosConfiguration,
    status_tx: tokio::sync::mpsc::UnboundedSender<ConfigurationStatus>,
    verbosity: Verbosity,
) -> Result<()> {
    let config_name = config.name.clone();

    // Step 1: Build the configuration
    let _ = status_tx.send(ConfigurationStatus::Building);

    let builder = Builder::new(config.deployment_config.effective_build_host());
    let out_path = match builder
        .build_configuration(
            &config,
            config
                .deployment_config
                .timeout_secs
                .map(std::time::Duration::from_secs),
        )
        .await
    {
        Ok(path) => path,
        Err(e) => {
            let error_msg = format!("Build failed: {}", e);
            let error_lines = e.get_error_tail(5);
            let _ = status_tx.send(ConfigurationStatus::Error {
                error: error_msg.clone(),
                error_lines,
            });
            return Err(e);
        }
    };

    let drv_path = config.drv_path.clone();
    let updated_config = config.with_paths(drv_path, Some(out_path.clone()));

    let _ = status_tx.send(ConfigurationStatus::Built {
        result: out_path.clone(),
    });

    // Step 2: Copy closure if needed
    let build_host = updated_config.deployment_config.effective_build_host();
    let target_host = &updated_config.deployment_config.target_host;

    let needs_copy = match (build_host, target_host) {
        (types::ExecutionTarget::Local, types::ExecutionTarget::Local) => false,
        (
            types::ExecutionTarget::Ssh { host: h1, .. },
            types::ExecutionTarget::Ssh { host: h2, .. },
        ) => h1 != h2,
        _ => true,
    };

    if needs_copy {
        let _ = status_tx.send(ConfigurationStatus::CopyingClosure);
        if let Err(e) = builder
            .copy_closure_to(
                &out_path,
                target_host,
                updated_config
                    .deployment_config
                    .timeout_secs
                    .map(std::time::Duration::from_secs),
            )
            .await
        {
            let error_msg = format!("Copy closure failed: {}", e);
            let error_lines = e.get_error_tail(5);
            let _ = status_tx.send(ConfigurationStatus::Error {
                error: error_msg.clone(),
                error_lines,
            });
            return Err(e);
        }
    }

    // Step 3: Deploy the configuration
    let _ = status_tx.send(ConfigurationStatus::Activating);

    let deployer = Deployer::new(target_host);
    let error_lines = match deployer
        .activate_configuration(
            &updated_config,
            verbosity,
            updated_config
                .deployment_config
                .timeout_secs
                .map(std::time::Duration::from_secs),
        )
        .await
    {
        Ok(lines) => lines,
        Err(e) => {
            let error_msg = format!("Activation failed: {}", e);
            let error_lines = e.get_error_tail(5);
            let _ = status_tx.send(ConfigurationStatus::Error {
                error: error_msg.clone(),
                error_lines,
            });
            return Err(e);
        }
    };

    if error_lines.is_empty() {
        let _ = status_tx.send(ConfigurationStatus::Activated);
    } else {
        // Deployment succeeded but with warnings
        if matches!(verbosity, Verbosity::Detailed) {
            for line in &error_lines {
                eprintln!("[{}] WARN: {}", config_name, line);
            }
        }
        let _ = status_tx.send(ConfigurationStatus::Activated);
    }

    if matches!(verbosity, Verbosity::Detailed) {
        println!(
            "[{}] INFO: Successfully deployed configuration",
            config_name
        );
    }
    Ok(())
}
