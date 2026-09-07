use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::sync::Semaphore;

use crate::config::{NetworkConfig, RequestFormat, RunConfig};
use crate::output::{JsonlWriter, ResultRecord};
use crate::payload::{Generator, validate_wordlists};
use crate::request::{parse_raw, parse_toml, placeholders};
use crate::transport::{ClientGeneration, ClientPool};

use super::{StopPolicy, StopState, WorkerCoordinator};

fn stop_requested(decision: &crate::rules::Decision, policy: StopPolicy) -> bool {
    !decision.matched.is_empty() && (policy.stop_on_match || decision.stop)
}

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
    let clients = Arc::new(ClientPool::new(&config.http, Default::default()));
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
    let jsonl_path = body_directory.join("results.jsonl");
    let output = Arc::new(
        JsonlWriter::create(&jsonl_path)
            .with_context(|| format!("failed to create JSONL output {}", jsonl_path.display()))?,
    );
    let stop_state = Arc::new(Mutex::new(StopState::new()));
    let stopping = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(AtomicU64::new(0));
    let matches = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicU64::new(0));
    let semaphore = Arc::new(Semaphore::new(config.execution.concurrency as usize));
    let mut tasks = Vec::with_capacity(config.execution.concurrency as usize);
    let mut index = 0_u64;

    while let Some(payload) = payloads.next_payload().context("failed to read payload")? {
        if stopping.load(Ordering::Acquire) {
            break;
        }
        let permit = Arc::clone(&semaphore)
            .acquire_owned()
            .await
            .context("run concurrency semaphore was closed")?;
        let worker = (index as usize) % config.execution.workers as usize;
        let task = execute_payload(
            Arc::clone(&clients),
            coordinator.clone(),
            Arc::clone(&output),
            Arc::clone(&stop_state),
            Arc::clone(&stopping),
            Arc::clone(&requests),
            Arc::clone(&matches),
            Arc::clone(&errors),
            policy,
            source.clone(),
            config.request.format.clone(),
            config.request.base_url.clone(),
            config.rules.clone(),
            body_directory.clone(),
            payload,
            index,
            worker,
            permit,
        );
        tasks.push(tokio::spawn(task));
        index += 1;
        if tasks.len() == config.execution.concurrency as usize {
            join_tasks(&mut tasks).await?;
        }
    }
    join_tasks(&mut tasks).await?;

    Ok(format!(
        "run completed: {} request(s), {} match(es), {} error(s); JSONL {}",
        requests.load(Ordering::Relaxed),
        matches.load(Ordering::Relaxed),
        errors.load(Ordering::Relaxed),
        jsonl_path.display()
    ))
}

async fn join_tasks(tasks: &mut Vec<tokio::task::JoinHandle<Result<()>>>) -> Result<()> {
    for task in tasks.drain(..) {
        task.await.context("run worker task panicked")??;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn execute_payload(
    clients: Arc<ClientPool>,
    coordinator: WorkerCoordinator,
    output: Arc<JsonlWriter>,
    stop_state: Arc<Mutex<StopState>>,
    stopping: Arc<AtomicBool>,
    requests: Arc<AtomicU64>,
    matches: Arc<AtomicU64>,
    errors: Arc<AtomicU64>,
    policy: StopPolicy,
    source: String,
    format: RequestFormat,
    base_url: Option<String>,
    rules: Vec<crate::rules::Rule>,
    body_directory: PathBuf,
    payload: std::collections::BTreeMap<String, String>,
    index: u64,
    worker: usize,
    _permit: tokio::sync::OwnedSemaphorePermit,
) -> Result<()> {
    let started = Instant::now();
    let address = coordinator.next_address(worker)?;
    let request = match format {
        RequestFormat::Raw => parse_raw(&source, base_url.as_deref(), &payload),
        RequestFormat::Toml => parse_toml(&source, base_url.as_deref(), &payload),
    }
    .context("failed to prepare request")?;
    let client = clients.client_for(worker, ClientGeneration(index), address.into())?;
    let result = client
        .execute_with_rules(request, &rules, &body_directory, MAX_RESPONSE_BODY_BYTES)
        .await;
    let (status, decision, body_file, error) = match result {
        Ok((status, decision, body_file)) => (Some(status), decision, body_file, None),
        Err(error) => {
            tracing::warn!(index, worker, error = %error, "request failed");
            (
                None,
                crate::rules::Decision::default(),
                None,
                Some(error.to_string()),
            )
        }
    };
    let transport_error = error.is_some();
    if transport_error {
        errors.fetch_add(1, Ordering::Relaxed);
    }
    let matched = !decision.matched.is_empty();
    if matched {
        matches.fetch_add(1, Ordering::Relaxed);
    }
    requests.fetch_add(1, Ordering::Relaxed);
    coordinator.record_result(worker, Some(&decision), transport_error)?;
    let matched_rules: Vec<String> = decision.matched.iter().cloned().collect();
    output.write(&ResultRecord {
        index,
        worker,
        payload: &payload,
        status,
        duration_ms: started.elapsed().as_millis() as u64,
        source_ipv6: Some(address.to_string()),
        matched_rules: &matched_rules,
        body_file: body_file.map(|path| path.display().to_string()),
        error,
    })?;
    let should_stop = stop_requested(&decision, policy);
    let should_stop = stop_state
        .lock()
        .map_err(|_| anyhow::anyhow!("stop state mutex poisoned"))?
        .record(transport_error, should_stop, policy);
    if should_stop {
        stopping.store(true, Ordering::Release);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::MAX_RESPONSE_BODY_BYTES;

    #[test]
    fn response_body_limit_is_bounded() {
        assert_eq!(MAX_RESPONSE_BODY_BYTES, 10 * 1024 * 1024);
    }

    #[test]
    fn rule_match_stops_only_when_requested() {
        let mut decision = crate::rules::Decision::default();
        decision.matched.insert("success".to_owned());
        let policy = super::StopPolicy {
            max_requests: None,
            max_duration: None,
            max_errors: None,
            max_error_ratio: None,
            stop_on_match: false,
        };
        assert!(!super::stop_requested(&decision, policy));
        decision.stop = true;
        assert!(super::stop_requested(&decision, policy));
    }
}
