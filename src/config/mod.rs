use std::collections::BTreeMap;
use std::fs;
use std::net::Ipv6Addr;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

const CONFIG_VERSION: u32 = 1;
const MAX_WORKERS: u16 = 1024;

#[derive(Debug, Error)]
#[error("{0}")]
pub struct ConfigError(String);

impl ConfigError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkConfig {
    pub version: u32,
    pub network: NetworkSection,
    #[serde(default)]
    pub route: RouteSection,
    #[serde(default)]
    pub sysctl: SysctlSection,
    #[serde(default)]
    pub state: StateSection,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkSection {
    pub interface: String,
    pub prefix: String,
    pub loopback: String,
    #[serde(default)]
    pub backend: NdpBackend,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NdpBackend {
    #[default]
    Native,
    Ndppd,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteSection {
    #[serde(default = "default_route_table")]
    pub table: String,
}

impl Default for RouteSection {
    fn default() -> Self {
        Self {
            table: default_route_table(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SysctlSection {
    #[serde(default)]
    pub forwarding: bool,
    #[serde(default)]
    pub ip_nonlocal_bind: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateSection {
    #[serde(default = "default_state_root")]
    pub root: PathBuf,
}

impl Default for StateSection {
    fn default() -> Self {
        Self {
            root: default_state_root(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunConfig {
    pub version: u32,
    pub request: RequestSection,
    #[serde(default)]
    pub wordlists: BTreeMap<String, String>,
    #[serde(default)]
    pub payloads: PayloadSection,
    #[serde(default)]
    pub execution: ExecutionSection,
    #[serde(default)]
    pub http: HttpSection,
    #[serde(default)]
    pub rotation: RotationSection,
    #[serde(default)]
    pub stop: StopSection,
    #[serde(default)]
    pub rules: Vec<crate::rules::Rule>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestSection {
    #[serde(default = "default_request_format")]
    pub format: RequestFormat,
    pub file: PathBuf,
    pub base_url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequestFormat {
    Raw,
    Toml,
}

fn default_request_format() -> RequestFormat {
    RequestFormat::Raw
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadSection {
    #[serde(default)]
    pub mode: PayloadMode,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PayloadMode {
    #[default]
    Clusterbomb,
    Pitchfork,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionSection {
    #[serde(default = "default_workers")]
    pub workers: u16,
    #[serde(default)]
    pub address_mode: AddressMode,
}

impl Default for ExecutionSection {
    fn default() -> Self {
        Self {
            workers: default_workers(),
            address_mode: AddressMode::Worker,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AddressMode {
    #[default]
    Worker,
    Request,
    Pool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpSection {
    #[serde(default)]
    pub version: HttpVersion,
    #[serde(default)]
    pub fallback_to_http1: bool,
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_ms: u64,
    #[serde(default = "default_total_timeout")]
    pub total_timeout_ms: u64,
    #[serde(default)]
    pub redirects: RedirectPolicy,
    #[serde(default)]
    pub cookies: CookieScope,
}

impl Default for HttpSection {
    fn default() -> Self {
        Self {
            version: HttpVersion::Auto,
            fallback_to_http1: false,
            connect_timeout_ms: default_connect_timeout(),
            total_timeout_ms: default_total_timeout(),
            redirects: RedirectPolicy::None,
            cookies: CookieScope::Worker,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HttpVersion {
    #[default]
    Auto,
    Http1,
    Http2,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RedirectPolicy {
    #[default]
    None,
    Limited,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CookieScope {
    None,
    #[default]
    Worker,
    Session,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RotationSection {
    #[serde(default)]
    pub scope: RotationScope,
    #[serde(default)]
    pub connection_policy: ConnectionPolicy,
    #[serde(default = "default_drain_timeout")]
    pub drain_timeout_ms: u64,
    pub every_requests: Option<u64>,
    pub every_ms: Option<u64>,
}

impl Default for RotationSection {
    fn default() -> Self {
        Self {
            scope: RotationScope::Worker,
            connection_policy: ConnectionPolicy::Drain,
            drain_timeout_ms: default_drain_timeout(),
            every_requests: None,
            every_ms: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RotationScope {
    #[default]
    Worker,
    Global,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionPolicy {
    #[default]
    Drain,
    Close,
    NextConnection,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopSection {
    pub max_requests: Option<u64>,
    pub max_duration_ms: Option<u64>,
    pub max_errors: Option<u64>,
    pub max_error_ratio: Option<f64>,
    #[serde(default)]
    pub stop_on_match: bool,
}

pub fn load_network(path: &Path) -> Result<NetworkConfig, ConfigError> {
    let contents = read_file(path)?;
    let config = toml::from_str::<NetworkConfig>(&contents).map_err(|error| {
        ConfigError::new(format!(
            "{}: failed to parse network configuration: {error}",
            path.display()
        ))
    })?;
    validate_network(&config, path)?;
    Ok(config)
}

pub fn validate_network_config(config: &NetworkConfig) -> Result<(), ConfigError> {
    validate_network(config, Path::new("<cli>"))
}

pub fn parse_ndp_backend(value: &str) -> Result<NdpBackend, ConfigError> {
    match value {
        "native" => Ok(NdpBackend::Native),
        "ndppd" => Ok(NdpBackend::Ndppd),
        other => Err(ConfigError::new(format!(
            "<cli> [network.backend]: expected 'native' or 'ndppd', got '{other}'"
        ))),
    }
}

pub fn load_run(path: &Path) -> Result<RunConfig, ConfigError> {
    let contents = read_file(path)?;
    let config = toml::from_str::<RunConfig>(&contents).map_err(|error| {
        ConfigError::new(format!(
            "{}: failed to parse run configuration: {error}",
            path.display()
        ))
    })?;
    validate_run(&config, path)?;
    Ok(config)
}

fn read_file(path: &Path) -> Result<String, ConfigError> {
    fs::read_to_string(path).map_err(|error| {
        ConfigError::new(format!("{}: failed to read file: {error}", path.display()))
    })
}

fn validate_network(config: &NetworkConfig, path: &Path) -> Result<(), ConfigError> {
    validate_version(config.version, path, "network")?;
    validate_non_empty(&config.network.interface, path, "network", "interface")?;
    validate_prefix(&config.network.prefix, path)?;
    validate_non_empty(&config.network.loopback, path, "network", "loopback")?;
    if config.route.table != "local" {
        return Err(ConfigError::new(format!(
            "{} [route.table]: expected 'local'",
            path.display()
        )));
    }
    validate_non_empty(&config.state.root.to_string_lossy(), path, "state", "root")
}

fn validate_run(config: &RunConfig, path: &Path) -> Result<(), ConfigError> {
    validate_version(config.version, path, "run")?;
    validate_non_empty(
        &config.request.file.to_string_lossy(),
        path,
        "request",
        "file",
    )?;
    validate_url(&config.request.base_url, path)?;
    if config.execution.workers == 0 || config.execution.workers > MAX_WORKERS {
        return Err(ConfigError::new(format!(
            "{} [execution.workers]: expected a value from 1 to {MAX_WORKERS}",
            path.display()
        )));
    }
    if config.http.connect_timeout_ms == 0 {
        return Err(ConfigError::new(format!(
            "{} [http.connect_timeout_ms]: must be greater than zero",
            path.display()
        )));
    }
    if config.http.total_timeout_ms == 0 {
        return Err(ConfigError::new(format!(
            "{} [http.total_timeout_ms]: must be greater than zero",
            path.display()
        )));
    }
    if config.rotation.every_requests == Some(0) || config.rotation.every_ms == Some(0) {
        return Err(ConfigError::new(format!(
            "{} [rotation]: intervals must be greater than zero",
            path.display()
        )));
    }
    if config.stop.max_duration_ms == Some(0)
        || config.stop.max_requests == Some(0)
        || config.stop.max_errors == Some(0)
        || config
            .stop
            .max_error_ratio
            .is_some_and(|ratio| !(0.0..=1.0).contains(&ratio))
    {
        return Err(ConfigError::new(format!(
            "{} [stop]: limits must be positive and error ratio must be between 0 and 1",
            path.display()
        )));
    }
    for rule in &config.rules {
        rule.when.validate().map_err(|error| {
            ConfigError::new(format!(
                "{} [rules.when]: invalid regular expression: {error}",
                path.display()
            ))
        })?;
    }
    Ok(())
}

fn validate_version(version: u32, path: &Path, section: &str) -> Result<(), ConfigError> {
    if version != CONFIG_VERSION {
        return Err(ConfigError::new(format!(
            "{} [{section}.version]: unsupported version {version}; expected {CONFIG_VERSION}",
            path.display()
        )));
    }
    Ok(())
}

fn validate_non_empty(
    value: &str,
    path: &Path,
    section: &str,
    field: &str,
) -> Result<(), ConfigError> {
    if value.trim().is_empty() {
        return Err(ConfigError::new(format!(
            "{} [{section}.{field}]: must not be empty",
            path.display()
        )));
    }
    Ok(())
}

fn validate_prefix(prefix: &str, path: &Path) -> Result<(), ConfigError> {
    let Some((address, length)) = prefix.split_once('/') else {
        return Err(ConfigError::new(format!(
            "{} [network.prefix]: expected an IPv6 prefix with a length from /64 to /128",
            path.display()
        )));
    };
    address.parse::<Ipv6Addr>().map_err(|_| {
        ConfigError::new(format!(
            "{} [network.prefix]: invalid IPv6 address",
            path.display()
        ))
    })?;
    let prefix_length = length.parse::<u8>().map_err(|_| {
        ConfigError::new(format!(
            "{} [network.prefix]: invalid prefix length",
            path.display()
        ))
    })?;
    if !(64..=128).contains(&prefix_length) {
        return Err(ConfigError::new(format!(
            "{} [network.prefix]: only prefix lengths from /64 to /128 are supported",
            path.display()
        )));
    }
    Ok(())
}

fn validate_url(url: &str, path: &Path) -> Result<(), ConfigError> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(ConfigError::new(format!(
            "{} [request.base_url]: expected an http:// or https:// URL",
            path.display()
        )));
    }
    Ok(())
}

fn default_route_table() -> String {
    "local".to_owned()
}

fn default_state_root() -> PathBuf {
    PathBuf::from("/run/aethel")
}

fn default_workers() -> u16 {
    1
}

fn default_connect_timeout() -> u64 {
    5_000
}

fn default_total_timeout() -> u64 {
    30_000
}

fn default_drain_timeout() -> u64 {
    5_000
}

#[cfg(test)]
mod tests {
    use super::*;

    const NETWORK: &str = r#"
version = 1

[network]
interface = "net0"
prefix = "2001:db8:1234:1::/64"
loopback = "lo"

[sysctl]
forwarding = true
ip_nonlocal_bind = true
"#;

    const RUN: &str = r#"
version = 1

[request]
format = "raw"
file = "request.http"
base_url = "https://example.test"
"#;

    #[test]
    fn valid_network_uses_explicit_defaults() {
        let config: NetworkConfig = toml::from_str(NETWORK).expect("valid network fixture");
        validate_network(&config, Path::new("network.toml")).expect("valid network");

        assert!(matches!(config.network.backend, NdpBackend::Native));
        assert_eq!(config.route.table, "local");
        assert_eq!(config.state.root, PathBuf::from("/run/aethel"));
    }

    #[test]
    fn valid_run_uses_explicit_defaults() {
        let config: RunConfig = toml::from_str(RUN).expect("valid run fixture");
        validate_run(&config, Path::new("run.toml")).expect("valid run");

        assert_eq!(config.execution.workers, 1);
        assert!(matches!(config.execution.address_mode, AddressMode::Worker));
        assert!(matches!(config.http.version, HttpVersion::Auto));
        assert!(!config.http.fallback_to_http1);
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let error = toml::from_str::<RunConfig>(
            "version = 1\n[request]\nfile = 'request.http'\nbase_url = 'https://example.test'\nunknown = true",
        )
        .expect_err("unknown field must fail");

        assert!(error.to_string().contains("unknown"));
    }

    #[test]
    fn incompatible_version_includes_file_and_field() {
        let config: NetworkConfig = toml::from_str(
            "version = 2\n[network]\ninterface = 'net0'\nprefix = '2001:db8::/64'\nloopback = 'lo'",
        )
        .expect("shape is valid");
        let error =
            validate_network(&config, Path::new("network.toml")).expect_err("version must fail");

        assert_eq!(
            error.to_string(),
            "network.toml [network.version]: unsupported version 2; expected 1"
        );
    }

    #[test]
    fn accepts_prefixes_from_64_through_128() {
        validate_prefix("2001:db8::/70", Path::new("network.toml"))
            .expect("/70 should be supported");
        validate_prefix("2001:db8::1/128", Path::new("network.toml"))
            .expect("/128 should be supported");
    }

    #[test]
    fn rejects_prefixes_broader_than_64() {
        let error = validate_prefix("2001:db8::/63", Path::new("network.toml"))
            .expect_err("/63 should be rejected");

        assert!(error.to_string().contains("from /64 to /128"));
    }

    #[test]
    fn invalid_enum_is_rejected() {
        let error = toml::from_str::<NetworkConfig>(
            "version = 1\n[network]\ninterface = 'net0'\nprefix = '2001:db8::/64'\nloopback = 'lo'\nbackend = 'unknown'",
        )
        .expect_err("invalid backend must fail");

        assert!(error.to_string().contains("backend"));
    }
}
