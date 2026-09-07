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
    /// Run a finite HTTP payload job.
    Run {
        /// Optional run TOML; CLI flags override values from this file.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Optional network TOML; omitted uses Linux network discovery.
        #[arg(long)]
        network: Option<PathBuf>,
        /// Base URL with scheme, required without an absolute request target.
        #[arg(short = 'u', long)]
        url: Option<String>,
        /// HTTP request template file.
        #[arg(short = 'r', long)]
        request: Option<PathBuf>,
        /// Repeated NAME:PATH wordlist mapping.
        #[arg(short = 'w', long = "wordlist")]
        wordlists: Vec<String>,
        /// Maximum simultaneous requests; independent from worker count.
        #[arg(short = 'c', long)]
        concurrency: Option<u16>,
        /// Number of worker state machines.
        #[arg(long)]
        workers: Option<u16>,
        /// Repeated stop condition NAME=VALUE.
        #[arg(short = 's', long = "stop")]
        stops: Vec<String>,
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
    /// Remove the local AnyIP route owned by this execution.
    Cleanup(NetworkArguments),
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
