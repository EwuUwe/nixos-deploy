#![warn(clippy::pedantic)]
#![allow(clippy::assigning_clones)]

use std::sync::Arc;

use crate::{
    cli::Commands,
    executor::LocalHost,
    nix::flake::NixFlake,
    pipeline::Stage,
};
use clap::Parser;
use color_eyre::Result;

mod cli;
mod executor;
mod host;
mod nix;
mod pipeline;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let args = cli::Cli::parse();

    let localhost = Arc::new(LocalHost);
    let flake = NixFlake {
        flake_ref: args.flake,
        executor: localhost,
    };

    let (stage, common) = match args.command {
        Commands::Eval { common } => (Stage::Eval, common),
        Commands::Build { common } => (Stage::Build, common),
        Commands::Push { common } => (Stage::Push, common),
        Commands::Apply { common } => (Stage::Apply, common),
        Commands::Show {} => todo!(),
    };

    let targets = pipeline::resolve_targets(&flake, &common.hosts).await?;
    pipeline::deploy(stage, targets).await?;

    Ok(())
}
