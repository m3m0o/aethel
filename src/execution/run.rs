use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::config::{NetworkConfig, RequestFormat, RunConfig};
use crate::payload::{Generator, validate_wordlists};
use crate::request::{parse_raw, parse_toml, placeholders};
use crate::rules::Decision;
use crate::transport::{ClientGeneration, ClientPool};

use super::{StopPolicy, StopState, WorkerCoordinator};

const MAX_RESPONSE_BODY_BYTES: u64 = 10 * 1024 * 1024;

pub async fn execute_run(config: &RunConfig, network: &NetworkConfig) -> Result<String> {
    let source = fs::read_to_string(&config.request.file).with_context(|| {
        format!(
            "failed to read request template {}",
            config.request.file.display()
        )
    })?;
    let names = placeholders(&source).context("failed to inspect request placeholders")?;
    let wordlists = config
        .wordlists
        .iter()
        .map(|(name, path)| (name.clone(), Path::new(path).to_path_buf()))
        .collect();
    validate_wordlists(&names, &wordlists).context("invalid run wordlists")?;
    let mut payloads = Generator::open(config.payloads.mode.clone(), wordlists)
        .context("failed to open run wordlists")?;
    let coordinator = WorkerCoordinator::new(
        &network.network.prefix,
        config.execution.address_mode.clone(),
        config.execution.workers,
        config.rotation.scope.clone(),
        config.rotation.every_requests,
        config.rotation.every_ms,
    )?;
    let clients = ClientPool::new(&config.http, Default::default());
    let policy = StopPolicy {
        max_requests: config.stop.max_requests,
        max_duration: config.stop.max_duration_ms.map(Duration::from_millis),
        max_errors: config.stop.max_errors,
        max_error_ratio: config.stop.max_error_ratio,
        stop_on_match: config.stop.stop_on_match,
    };
    let body_directory = config
        .request
        .file
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("results");
    let mut state = StopState::new();
    let mut index = 0_u64;
    let mut matches = 0_u64;
    let mut errors = 0_u64;

    while let Some(payload) = payloads.next_payload().context("failed to read payload")? {
        let worker = (index as usize) % config.execution.workers as usize;
        let address = coordinator.next_address(worker)?;
        let request = match config.request.format {
            RequestFormat::Raw => parse_raw(&source, config.request.base_url.as_deref(), &payload),
            RequestFormat::Toml => {
                parse_toml(&source, config.request.base_url.as_deref(), &payload)
            }
        };
        let request = request.context("failed to prepare request")?;
        let client = clients.client_for(worker, ClientGeneration(index), address.into())?;
        let result = client
            .execute_with_rules(
                request,
                &config.rules,
                &body_directory,
                MAX_RESPONSE_BODY_BYTES,
            )
            .await;
        let (decision, body_file, transport_error) = match result {
            Ok((decision, body_file)) => (decision, body_file, false),
            Err(error) => {
                errors += 1;
                tracing::warn!(index, worker, error = %error, "request failed");
                (Decision::default(), None, true)
            }
        };
        if !decision.matched.is_empty() {
            matches += 1;
        }
        coordinator.record_result(worker, Some(&decision), transport_error)?;
        let matched = !decision.matched.is_empty();
        index += 1;
        if state.record(transport_error, matched, policy) {
            break;
        }
        let _ = body_file;
    }

    Ok(format!(
        "run completed: {} request(s), {} match(es), {} error(s)",
        index, matches, errors
    ))
}

#[cfg(test)]
mod tests {
    use super::MAX_RESPONSE_BODY_BYTES;

    #[test]
    fn response_body_limit_is_bounded() {
        assert_eq!(MAX_RESPONSE_BODY_BYTES, 10 * 1024 * 1024);
    }
}
