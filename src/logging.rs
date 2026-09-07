use std::env;

use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::{EnvFilter, fmt};

pub fn init() {
    let filter = if env::var_os("RUST_LOG").is_some() {
        EnvFilter::try_from_default_env().unwrap_or_else(|error| {
            eprintln!("warning: invalid RUST_LOG filter ({error}); using aethel=info");
            default_filter()
        })
    } else {
        default_filter()
    };

    if env::var("AETHEL_LOG_FORMAT").as_deref() == Ok("json") {
        fmt()
            .json()
            .with_env_filter(filter)
            .with_target(true)
            .init();
    } else {
        fmt().with_env_filter(filter).with_target(true).init();
    }
}

fn default_filter() -> EnvFilter {
    EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy()
}
