use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use reqwest::header::{HeaderName, HeaderValue};

use super::model::PreparedRequest;
use super::substitute::substitute;

pub fn parse(
    raw: &str,
    base_url: &str,
    values: &BTreeMap<String, String>,
) -> Result<PreparedRequest> {
    let (head, body) = raw
        .split_once("\r\n\r\n")
        .or_else(|| raw.split_once("\n\n"))
        .ok_or_else(|| anyhow::anyhow!("raw HTTP template must contain a blank line"))?;
    let mut lines = head.lines();
    let request_line = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("raw HTTP template is empty"))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("raw HTTP request line is missing method"))?
        .parse::<reqwest::Method>()
        .context("invalid HTTP method")?;
    let target = request_parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("raw HTTP request line is missing target"))?;
    let version = request_parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("raw HTTP request line is missing version"))?;
    if request_parts.next().is_some() || version != "HTTP/1.1" {
        bail!("raw HTTP request line must contain method, target, and HTTP/1.1");
    }

    let mut headers = reqwest::header::HeaderMap::new();
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("invalid raw HTTP header line: {line}"))?;
        let name = HeaderName::from_bytes(name.trim().as_bytes()).context("invalid header name")?;
        let value = substitute(value.trim(), values)?;
        if value.contains(['\r', '\n']) {
            bail!("header value contains CR/LF");
        }
        if headers.contains_key(&name) {
            bail!("duplicate header: {name}");
        }
        headers.insert(
            name,
            HeaderValue::from_str(&value).context("invalid header value")?,
        );
    }

    let target = substitute(target, values)?;
    let base_url = reqwest::Url::parse(base_url).context("invalid base URL")?;
    let url = base_url
        .join(&target)
        .context("invalid request target URL")?;
    let body = substitute(body, values)?.into_bytes();
    Ok(PreparedRequest {
        method,
        url,
        headers,
        body,
    })
}
