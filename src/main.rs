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
use config::{
    ExecutionSection, NetworkConfig, PayloadSection, RequestFormat, RequestSection, RunConfig,
    StopSection, load_network, load_run, parse_ndp_backend, validate_network_config,
};
use error::AppError;
use network::{cleanup, configured_summary, discover, host_summary, setup};
use std::path::PathBuf;
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
            url,
            request,
            wordlists,
            concurrency,
            workers,
            stops,
        } => {
            let mut run = match config {
                Some(path) => load_run(&path).with_context(|| {
                    format!("failed to load run configuration: {}", path.display())
                })?,
                None => build_cli_run(request, url)?,
            };
            if let Some(path) = request {
                run.request.file = path;
            }
            if let Some(url) = url {
                run.request.base_url = Some(url);
            }
            for mapping in wordlists {
                let (name, path) = mapping.split_once(':').ok_or_else(|| {
                    anyhow::anyhow!("invalid --wordlist '{mapping}'; expected NAME:PATH")
                })?;
                if name.is_empty() || path.is_empty() {
                    anyhow::bail!("invalid --wordlist '{mapping}'; expected NAME:PATH");
                }
                run.wordlists.insert(name.to_owned(), path.to_owned());
            }
            if let Some(concurrency) = concurrency {
                run.execution.concurrency = concurrency;
            }
            if let Some(workers) = workers {
                run.execution.workers = workers;
            }
            apply_stop_flags(&mut run.stop, &stops)?;
            let network = match network {
                Some(path) => load_network(&path).with_context(|| {
                    format!("failed to load network configuration: {}", path.display())
                })?,
                None => discover().context("failed to discover network configuration")?,
            };
            validate_network_config(&network)?;
            Ok(format!(
                "run configuration valid: {} worker(s), {} concurrent request(s), network {}",
                run.execution.workers, run.execution.concurrency, network.network.prefix
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

fn build_cli_run(request: Option<PathBuf>, url: Option<String>) -> Result<RunConfig, AppError> {
    let request =
        request.ok_or_else(|| anyhow::anyhow!("--request is required without --config"))?;
    Ok(RunConfig {
        version: 1,
        request: RequestSection {
            format: RequestFormat::Raw,
            file: request,
            base_url: url,
        },
        wordlists: Default::default(),
        payloads: PayloadSection::default(),
        execution: ExecutionSection::default(),
        http: Default::default(),
        rotation: Default::default(),
        stop: StopSection::default(),
        rules: Vec::new(),
    })
}

fn apply_stop_flags(stop: &mut StopSection, flags: &[String]) -> Result<(), AppError> {
    for flag in flags {
        let (name, value) = flag
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("invalid --stop '{flag}'; expected NAME=VALUE"))?;
        match name {
            "max-requests" => stop.max_requests = Some(value.parse()?),
            "max-duration-ms" => stop.max_duration_ms = Some(value.parse()?),
            "max-errors" => stop.max_errors = Some(value.parse()?),
            "max-error-ratio" => stop.max_error_ratio = Some(value.parse()?),
            "on-match" => {
                stop.stop_on_match = value.parse().map_err(|_| {
                    anyhow::anyhow!("invalid --stop '{flag}'; on-match expects true or false")
                })?
            }
            _ => anyhow::bail!("unknown stop condition '{name}'"),
        }
    }
    Ok(())
}
