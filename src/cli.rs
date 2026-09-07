use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "aethel",
    version,
    about = "IPv6 AnyIP HTTP automation CLI",
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Validate a run configuration and its network configuration.
    Run {
        /// Path to the run TOML file.
        #[arg(long, value_name = "FILE")]
        config: PathBuf,
        /// Path to the network TOML file.
        #[arg(long, value_name = "FILE")]
        network: PathBuf,
        /// Override the worker count from the run configuration.
        #[arg(long, value_name = "COUNT")]
        workers: Option<u16>,
    },
    /// Inspect or change the configured network.
    Network {
        #[command(subcommand)]
        command: NetworkCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum NetworkCommand {
    /// Validate the network configuration before setup.
    Setup {
        /// Path to the network TOML file.
        #[arg(long, value_name = "FILE")]
        config: PathBuf,
    },
    /// Inspect current network state, optionally against a network TOML file.
    Check {
        /// Optional path to the expected network TOML file.
        #[arg(long, value_name = "FILE")]
        config: Option<PathBuf>,
    },
    /// Validate the network configuration before cleanup.
    Cleanup {
        /// Path to the network TOML file.
        #[arg(long, value_name = "FILE")]
        config: PathBuf,
    },
}
