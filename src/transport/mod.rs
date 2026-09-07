mod connection;

use anyhow::{Context, Result};
pub use connection::{ConnectionLifecycle, RequestAdmission, RequestGuard};
use futures_util::StreamExt;
use reqwest::header::HeaderMap;
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::{CookieScope, HttpSection, HttpVersion, RedirectPolicy};
use crate::request::PreparedRequest;
use crate::rules::{BodyStore, Decision, Response as RuleResponse, Rule, evaluate};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientGeneration(pub u64);

pub struct ClientPool {
    config: HttpSection,
    default_headers: HeaderMap,
    clients: Mutex<HashMap<usize, Arc<SourceBoundClient>>>,
}

impl ClientPool {
    pub fn new(config: &HttpSection, default_headers: HeaderMap) -> Self {
        Self {
            config: config.clone(),
            default_headers,
            clients: Mutex::new(HashMap::new()),
        }
    }

    pub fn client_for(
        &self,
        worker: usize,
        generation: ClientGeneration,
        address: IpAddr,
    ) -> Result<Arc<SourceBoundClient>> {
        if matches!(self.config.cookies, CookieScope::None) {
            return Ok(Arc::new(SourceBoundClient::new(
                generation,
                address,
                &self.config,
                self.default_headers.clone(),
            )?));
        }
        let key = match self.config.cookies {
            CookieScope::Worker => worker,
            CookieScope::Session => 0,
            CookieScope::None => unreachable!(),
        };
        let mut clients = self.clients.lock().expect("client pool mutex poisoned");
        if let Some(client) = clients.get(&key) {
            if client.generation() == generation && client.local_address() == address {
                return Ok(Arc::clone(client));
            }
        }
        let client = Arc::new(SourceBoundClient::new(
            generation,
            address,
            &self.config,
            self.default_headers.clone(),
        )?);
        clients.insert(key, Arc::clone(&client));
        Ok(client)
    }
}

pub struct SourceBoundClient {
    generation: ClientGeneration,
    local_address: IpAddr,
    reject_http1_fallback: bool,
    client: reqwest::Client,
}

impl SourceBoundClient {
    pub fn new(
        generation: ClientGeneration,
        local_address: IpAddr,
        config: &HttpSection,
        default_headers: HeaderMap,
    ) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .local_address(Some(local_address))
            .connect_timeout(Duration::from_millis(config.connect_timeout_ms))
            .timeout(Duration::from_millis(config.total_timeout_ms))
            .default_headers(default_headers)
            .cookie_store(!matches!(config.cookies, CookieScope::None))
            .redirect(match config.redirects {
                RedirectPolicy::None => reqwest::redirect::Policy::none(),
                RedirectPolicy::Limited => reqwest::redirect::Policy::limited(10),
            });

        builder = match config.version {
            HttpVersion::Auto => builder,
            HttpVersion::Http2 => builder.http2_prior_knowledge(),
            HttpVersion::Http1 => builder.http1_only(),
        };

        let client = builder
            .build()
            .context("failed to build source-bound HTTP client")?;
        Ok(Self {
            generation,
            local_address,
            reject_http1_fallback: matches!(config.version, HttpVersion::Auto)
                && !config.fallback_to_http1,
            client,
        })
    }

    pub fn generation(&self) -> ClientGeneration {
        self.generation
    }

    pub fn local_address(&self) -> IpAddr {
        self.local_address
    }

    pub async fn execute(&self, request: PreparedRequest) -> Result<reqwest::Response> {
        let response = self
            .client
            .request(request.method, request.url)
            .headers(request.headers)
            .body(request.body)
            .send()
            .await
            .with_context(|| {
                format!(
                    "HTTP request from {} failed; verify that the source address is configured locally or that net.ipv6.ip_nonlocal_bind is enabled",
                    self.local_address
                )
            })?;
        if self.reject_http1_fallback && response.version() == reqwest::Version::HTTP_11 {
            return Err(anyhow::anyhow!(
                "server negotiated HTTP/1.1 while HTTP/1.1 fallback is disabled"
            ));
        }
        Ok(response)
    }

    pub async fn execute_with_rules(
        &self,
        request: PreparedRequest,
        rules: &[Rule],
        body_directory: impl AsRef<Path>,
        max_body_bytes: u64,
    ) -> Result<(Decision, Option<std::path::PathBuf>)> {
        let response = self.execute(request).await?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let mut body_store = BodyStore::create(body_directory, max_body_bytes)
            .context("failed to create bounded response body store")?;
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("failed to read HTTP response body")?;
            body_store
                .write_chunk(&chunk)
                .context("response body exceeded configured limit")?;
            body.extend_from_slice(&chunk);
        }
        let rule_response = RuleResponse {
            status,
            headers: &headers,
            body: &body,
        };
        let decision = evaluate(rules, &rule_response)
            .map_err(|error| anyhow::anyhow!("invalid response rule: {error}"))?;
        let saved_body = body_store
            .finish(decision.save_body)
            .context("failed to finalize response body store")?;
        Ok((decision, saved_body))
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }
}

#[cfg(test)]
mod tests {
    use super::{ClientGeneration, ClientPool, SourceBoundClient};
    use crate::config::{CookieScope, HttpSection, HttpVersion, RedirectPolicy};
    use reqwest::header::{HeaderMap, HeaderValue};
    use std::net::{IpAddr, Ipv6Addr};

    fn config() -> HttpSection {
        HttpSection {
            version: HttpVersion::Auto,
            fallback_to_http1: true,
            connect_timeout_ms: 100,
            total_timeout_ms: 200,
            redirects: RedirectPolicy::Limited,
            cookies: CookieScope::Worker,
        }
    }

    #[test]
    fn creates_independent_clients_per_generation() {
        let address = IpAddr::V6(Ipv6Addr::LOCALHOST);
        let first =
            SourceBoundClient::new(ClientGeneration(1), address, &config(), HeaderMap::new())
                .unwrap();
        let second =
            SourceBoundClient::new(ClientGeneration(2), address, &config(), HeaderMap::new())
                .unwrap();
        assert_ne!(first.generation(), second.generation());
    }

    #[test]
    fn only_auto_mode_without_fallback_rejects_http1() {
        let address = IpAddr::V6(Ipv6Addr::LOCALHOST);
        let mut auto_config = config();
        auto_config.fallback_to_http1 = false;
        let auto =
            SourceBoundClient::new(ClientGeneration(1), address, &auto_config, HeaderMap::new())
                .unwrap();
        assert!(auto.reject_http1_fallback);

        let mut http1_config = config();
        http1_config.version = HttpVersion::Http1;
        http1_config.fallback_to_http1 = false;
        let http1 = SourceBoundClient::new(
            ClientGeneration(2),
            address,
            &http1_config,
            HeaderMap::new(),
        )
        .unwrap();
        assert!(!http1.reject_http1_fallback);
    }

    #[test]
    fn applies_default_headers_without_mutating_input() {
        let mut headers = HeaderMap::new();
        headers.insert("x-aethel", HeaderValue::from_static("test"));
        let client = SourceBoundClient::new(
            ClientGeneration(1),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            &config(),
            headers.clone(),
        )
        .unwrap();
        assert_eq!(client.generation(), ClientGeneration(1));
        assert_eq!(headers["x-aethel"], "test");
    }
    #[test]
    fn client_pool_reuses_scope_and_replaces_generation() {
        let address = IpAddr::V6(Ipv6Addr::LOCALHOST);
        let pool = ClientPool::new(&config(), HeaderMap::new());
        let first = pool.client_for(0, ClientGeneration(1), address).unwrap();
        let reused = pool.client_for(0, ClientGeneration(1), address).unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &reused));
        let replaced = pool.client_for(0, ClientGeneration(2), address).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&first, &replaced));
    }
}
