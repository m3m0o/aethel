use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

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
    /// Create the configured local AnyIP route.
    Setup(NetworkArguments),
    /// Inspect current network state.
    Check(NetworkArguments),
    /// Remove only network state owned by this execution after an interrupted run.
    Recover(NetworkArguments),
}

#[derive(Debug, Args)]
pub struct NetworkArguments {
    /// Optional path to the network TOML file.
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,
    /// Network interface used for IPv6 traffic.
    #[arg(long, value_name = "NAME")]
    pub interface: Option<String>,
    /// IPv6 prefix managed by AnyIP.
    #[arg(long, value_name = "PREFIX")]
    pub prefix: Option<String>,
    /// Loopback interface receiving the local route.
    #[arg(long, value_name = "NAME")]
    pub loopback: Option<String>,
    /// NDP backend: native or ndppd.
    #[arg(long, value_name = "BACKEND")]
    pub backend: Option<String>,
    /// Runtime state directory.
    #[arg(long, value_name = "DIR")]
    pub state_root: Option<PathBuf>,
}
