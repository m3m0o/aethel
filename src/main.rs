mod cli;
pub mod config;
mod network;

use clap::Parser;
use cli::{Cli, Command, NetworkCommand};
use config::{load_network, load_run};
use network::host_summary;
use std::process::ExitCode;

fn main() -> ExitCode {
    match execute(Cli::parse()) {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}

fn execute(cli: Cli) -> Result<String, String> {
    match cli.command {
        Command::Run {
            config,
            network,
            workers,
        } => {
            let mut run = load_run(&config).map_err(|error| error.to_string())?;
            load_network(&network).map_err(|error| error.to_string())?;

            if let Some(workers) = workers {
                if workers == 0 || workers > 1024 {
                    return Err(format!(
                        "{} [--workers]: expected a value from 1 to 1024",
                        config.display()
                    ));
                }
                run.execution.workers = workers;
            }

            Ok(format!(
                "run configuration valid: {} worker(s)",
                run.execution.workers
            ))
        }
        Command::Network { command } => match command {
            NetworkCommand::Setup { config } | NetworkCommand::Cleanup { config } => {
                load_network(&config).map_err(|error| error.to_string())?;
                Ok(format!("network configuration valid: {}", config.display()))
            }
            NetworkCommand::Check {
                config: Some(config),
            } => {
                load_network(&config).map_err(|error| error.to_string())?;
                Ok(format!("network configuration valid: {}", config.display()))
            }
            NetworkCommand::Check { config: None } => host_summary(),
        },
    }
}
