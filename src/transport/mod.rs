use anyhow::{Context, Result};
use reqwest::header::HeaderMap;
use std::net::IpAddr;
use std::time::Duration;

use crate::config::{CookieScope, HttpSection, HttpVersion, RedirectPolicy};
use crate::request::PreparedRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientGeneration(pub u64);

pub struct SourceBoundClient {
    generation: ClientGeneration,
    local_address: IpAddr,
    fallback_to_http1: bool,
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
            HttpVersion::Http2 => builder.http2_only(),
            HttpVersion::Http1 => builder.http1_only(),
        };

        let client = builder
            .build()
            .context("failed to build source-bound HTTP client")?;
        Ok(Self {
            generation,
            local_address,
            fallback_to_http1: config.fallback_to_http1,
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
        if !self.fallback_to_http1 && response.version() == reqwest::Version::HTTP_11 {
            return Err(anyhow::anyhow!(
                "server negotiated HTTP/1.1 while HTTP/1.1 fallback is disabled"
            ));
        }
        Ok(response)
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }
}

#[cfg(test)]
mod tests {
    use super::{ClientGeneration, SourceBoundClient};
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
}
