mod cli;
pub mod config;
mod error;
mod logging;
mod network;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Command, NetworkCommand};
use config::{load_network, load_run};
use error::AppError;
use network::{cleanup, configured_summary, host_summary, setup};
use std::process::ExitCode;

fn main() -> ExitCode {
    logging::init();

    match execute(Cli::parse()) {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!(error = %error, "application failed");
            eprintln!("error: {error}");
            ExitCode::from(error.exit_code())
        }
    }
}

fn execute(cli: Cli) -> Result<String, AppError> {
    match cli.command {
        Command::Run {
            config,
            network,
            workers,
        } => {
            let mut run = load_run(&config).with_context(|| {
                format!("failed to load run configuration: {}", config.display())
            })?;
            load_network(&network).with_context(|| {
                format!(
                    "failed to load network configuration: {}",
                    network.display()
                )
            })?;

            if let Some(workers) = workers {
                if workers == 0 || workers > 1024 {
                    return Err(anyhow::anyhow!(
                        "{} [--workers]: expected a value from 1 to 1024",
                        config.display()
                    )
                    .into());
                }
                run.execution.workers = workers;
            }

            Ok(format!(
                "run configuration valid: {} worker(s)",
                run.execution.workers
            ))
        }
        Command::Network { command } => match command {
            NetworkCommand::Setup { config } => {
                let network = load_network(&config).with_context(|| {
                    format!("failed to load network configuration: {}", config.display())
                })?;
                setup(&network)
                    .with_context(|| format!("failed to set up network: {}", config.display()))
                    .map_err(AppError::from)
            }
            NetworkCommand::Cleanup { config } => {
                let network = load_network(&config).with_context(|| {
                    format!("failed to load network configuration: {}", config.display())
                })?;
                cleanup(&network)
                    .with_context(|| format!("failed to clean up network: {}", config.display()))
                    .map_err(AppError::from)
            }
            NetworkCommand::Check {
                config: Some(config),
            } => {
                let network = load_network(&config).with_context(|| {
                    format!("failed to load network configuration: {}", config.display())
                })?;
                configured_summary(&network)
                    .with_context(|| format!("failed to inspect network: {}", config.display()))
                    .map_err(AppError::from)
            }
            NetworkCommand::Check { config: None } => host_summary().map_err(AppError::from),
        },
    }
}
