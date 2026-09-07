use std::collections::BTreeMap;

use anyhow::{Context, Result};
use reqwest::header::{HeaderName, HeaderValue};
use serde::Deserialize;

use super::model::PreparedRequest;
use super::substitute::substitute;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Template {
    method: String,
    path: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body: String,
}

pub fn parse(
    source: &str,
    base_url: Option<&str>,
    values: &BTreeMap<String, String>,
) -> Result<PreparedRequest> {
    let template =
        toml::from_str::<Template>(source).context("failed to parse TOML request template")?;
    let method = template
        .method
        .parse::<reqwest::Method>()
        .context("invalid HTTP method")?;
    let path = substitute(&template.path, values)?;
    let url = match reqwest::Url::parse(&path) {
        Ok(url) if url.has_authority() => url,
        _ => {
            let base_url = base_url.ok_or_else(|| {
                anyhow::anyhow!("relative structured request path requires a base URL")
            })?;
            reqwest::Url::parse(base_url)
                .context("invalid base URL")?
                .join(&path)
                .context("invalid structured request path")?
        }
    };

    let mut headers = template.headers;
    let body = if template.body.is_empty() {
        headers.remove("body").unwrap_or_default()
    } else {
        template.body
    };
    let body = substitute(&body, values)?.into_bytes();

    let mut request_headers = reqwest::header::HeaderMap::new();
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes()).context("invalid header name")?;
        let value = substitute(&value, values)?;
        if value.contains(['\r', '\n']) {
            anyhow::bail!("header value contains CR/LF");
        }
        if request_headers.contains_key(&name) {
            anyhow::bail!("duplicate header: {name}");
        }
        request_headers.insert(
            name,
            HeaderValue::from_str(&value).context("invalid header value")?,
        );
    }

    Ok(PreparedRequest {
        method,
        url,
        headers: request_headers,
        body,
    })
}
