mod cli;
pub mod config;
mod error;
pub mod execution;
mod logging;
pub mod network;
pub mod output;
pub mod payload;
pub mod request;
pub mod rules;
pub mod transport;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Command, NetworkArguments, NetworkCommand};
use config::{NetworkConfig, load_network, load_run, parse_ndp_backend, validate_network_config};
use error::AppError;
use network::{cleanup, configured_summary, discover, host_summary, setup};
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
            eprintln!("error: {error:#}");
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
            NetworkCommand::Setup(arguments) => {
                let network = resolve_network(arguments)?;
                setup(&network)
                    .context("failed to set up network")
                    .map_err(AppError::from)
            }
            NetworkCommand::Cleanup(arguments) => {
                let network = resolve_network(arguments)?;
                cleanup(&network)
                    .context("failed to clean up network")
                    .map_err(AppError::from)
            }
            NetworkCommand::Check(arguments) => {
                if arguments.config.is_none()
                    && arguments.interface.is_none()
                    && arguments.prefix.is_none()
                    && arguments.loopback.is_none()
                    && arguments.backend.is_none()
                    && arguments.state_root.is_none()
                {
                    return host_summary().map_err(AppError::from);
                }
                let network = resolve_network(arguments)?;
                configured_summary(&network)
                    .context("failed to inspect network")
                    .map_err(AppError::from)
            }
        },
    }
}

fn resolve_network(arguments: NetworkArguments) -> Result<NetworkConfig, AppError> {
    let mut network = match arguments.config {
        Some(path) => load_network(&path)
            .with_context(|| format!("failed to load network configuration: {}", path.display()))?,
        None => discover().context("failed to discover network configuration")?,
    };

    if let Some(interface) = arguments.interface {
        network.network.interface = interface;
    }
    if let Some(prefix) = arguments.prefix {
        network.network.prefix = prefix;
    }
    if let Some(loopback) = arguments.loopback {
        network.network.loopback = loopback;
    }
    if let Some(backend) = arguments.backend {
        network.network.backend = parse_ndp_backend(&backend)?;
    }
    if let Some(state_root) = arguments.state_root {
        network.state.root = state_root;
    }

    validate_network_config(&network)?;
    Ok(network)
}
