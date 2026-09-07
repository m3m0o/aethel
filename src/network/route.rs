use std::fs::{self, OpenOptions};
use std::future::Future;
use std::io::Write;
use std::net::Ipv6Addr;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, Result};
use netlink_packet_route::route::{RouteScope, RouteType};
use rtnetlink::RouteMessageBuilder;

use crate::config::NetworkConfig;

use super::inspect::{interface_index, read_routes};

const ROUTE_MARKER: &str = "anyip-route.state";

pub fn setup(config: &NetworkConfig) -> Result<String> {
    let (prefix, prefix_length) = parse_prefix(&config.network.prefix)?;
    let capabilities = super::capabilities::inspect()?;
    if !capabilities.net_admin {
        anyhow::bail!(
            "CAP_NET_ADMIN is required to create the AnyIP route; run as root or grant the capability to the aethel binary"
        );
    }
    let loopback_index = interface_index(&config.network.loopback)?;
    let routes = read_routes()?;
    match find_route(&routes, prefix, prefix_length, loopback_index) {
        Some(true) => Ok(format!(
            "AnyIP route already exists: {}",
            config.network.prefix
        )),
        Some(false) => anyhow::bail!(
            "incompatible route exists for {}: expected local route on {}",
            config.network.prefix,
            config.network.loopback
        ),
        None => {
            remove_stale_marker(config)?;
            add_route(prefix, prefix_length, loopback_index)?;
            if let Err(error) = write_route_marker(config, prefix, prefix_length, loopback_index) {
                let _ = delete_route(prefix, prefix_length, loopback_index);
                return Err(error).context("failed to record AnyIP route ownership");
            }
            Ok(format!("AnyIP route created: {}", config.network.prefix))
        }
    }
}

pub fn cleanup(config: &NetworkConfig) -> Result<String> {
    let (prefix, prefix_length) = parse_prefix(&config.network.prefix)?;
    let loopback_index = interface_index(&config.network.loopback)?;
    let routes = read_routes()?;
    let Some(owned_route) = route_marker(config)? else {
        return Ok(format!(
            "AnyIP route left unchanged (not owned by this execution): {}",
            config.network.prefix
        ));
    };
    if owned_route != (prefix, prefix_length, loopback_index) {
        anyhow::bail!("route ownership marker does not match configured prefix or loopback");
    }

    match find_route(&routes, prefix, prefix_length, loopback_index) {
        Some(true) => {
            delete_route(prefix, prefix_length, loopback_index)?;
            remove_route_marker(config)?;
            Ok(format!("AnyIP route removed: {}", config.network.prefix))
        }
        Some(false) => anyhow::bail!(
            "refusing to remove incompatible route for {}",
            config.network.prefix
        ),
        None => {
            remove_route_marker(config)?;
            Ok(format!(
                "AnyIP route already absent: {}",
                config.network.prefix
            ))
        }
    }
}

pub fn recover(config: &NetworkConfig) -> Result<String> {
    cleanup(config).context("failed to recover network state")
}
fn parse_prefix(value: &str) -> Result<(Ipv6Addr, u8)> {
    let (address, length) = value
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("invalid IPv6 prefix: {value}"))?;
    let address = Ipv6Addr::from_str(address)
        .with_context(|| format!("invalid IPv6 prefix address: {address}"))?;
    let length = length
        .parse::<u8>()
        .with_context(|| format!("invalid IPv6 prefix length: {length}"))?;
    if length != 64 {
        anyhow::bail!("AnyIP route requires an IPv6 /64 prefix, got /{length}");
    }
    Ok((address, length))
}

fn find_route(
    routes: &[super::snapshot::RouteState],
    prefix: Ipv6Addr,
    prefix_length: u8,
    loopback_index: u32,
) -> Option<bool> {
    let destination = format!("{prefix}/{prefix_length}");
    routes
        .iter()
        .find(|route| route.destination.as_deref() == Some(destination.as_str()))
        .map(|route| {
            route.kind == "local"
                && route.output_interface.as_deref() == Some(&loopback_index.to_string())
        })
}

fn route_marker(config: &NetworkConfig) -> Result<Option<(Ipv6Addr, u8, u32)>> {
    let path = marker_path(config);
    let Ok(value) = fs::read_to_string(&path) else {
        return Ok(None);
    };
    let mut lines = value.lines();
    let prefix = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("invalid route ownership marker: {}", path.display()))?;
    let index = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("invalid route ownership marker: {}", path.display()))?;
    if lines.next().is_some() {
        anyhow::bail!("invalid route ownership marker: {}", path.display());
    }
    let (address, length) = parse_prefix(prefix)?;
    let index = index
        .parse::<u32>()
        .with_context(|| format!("invalid route ownership interface index: {index}"))?;
    Ok(Some((address, length, index)))
}

fn write_route_marker(
    config: &NetworkConfig,
    prefix: Ipv6Addr,
    prefix_length: u8,
    loopback_index: u32,
) -> Result<()> {
    fs::create_dir_all(&config.state.root).with_context(|| {
        format!(
            "failed to create state directory {}",
            config.state.root.display()
        )
    })?;
    let path = marker_path(config);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("failed to create route ownership marker {}", path.display()))?;
    writeln!(file, "{prefix}/{prefix_length}")?;
    writeln!(file, "{loopback_index}")?;
    Ok(())
}

fn remove_stale_marker(config: &NetworkConfig) -> Result<()> {
    match route_marker(config)? {
        Some(_) => remove_route_marker(config),
        None => Ok(()),
    }
}

fn remove_route_marker(config: &NetworkConfig) -> Result<()> {
    let path = marker_path(config);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to remove route ownership marker {}", path.display())),
    }
}

fn marker_path(config: &NetworkConfig) -> PathBuf {
    config.state.root.join(ROUTE_MARKER)
}

fn add_route(prefix: Ipv6Addr, prefix_length: u8, loopback_index: u32) -> Result<()> {
    run_netlink(async move {
        let (connection, handle, _) =
            rtnetlink::new_connection().context("failed to open route Netlink connection")?;
        tokio::spawn(connection);
        let route = RouteMessageBuilder::<Ipv6Addr>::new()
            .destination_prefix(prefix, prefix_length)
            .output_interface(loopback_index)
            .table_id(255)
            .kind(RouteType::Local)
            .scope(RouteScope::Host)
            .build();
        handle
            .route()
            .add(route)
            .execute()
            .await
            .context("failed to create AnyIP route")
    })
}

fn delete_route(prefix: Ipv6Addr, prefix_length: u8, loopback_index: u32) -> Result<()> {
    run_netlink(async move {
        let (connection, handle, _) =
            rtnetlink::new_connection().context("failed to open route Netlink connection")?;
        tokio::spawn(connection);
        let route = RouteMessageBuilder::<Ipv6Addr>::new()
            .destination_prefix(prefix, prefix_length)
            .output_interface(loopback_index)
            .table_id(255)
            .kind(RouteType::Local)
            .scope(RouteScope::Host)
            .build();
        handle
            .route()
            .del(route)
            .execute()
            .await
            .context("failed to remove AnyIP route")
    })
}

fn run_netlink<F>(operation: F) -> Result<()>
where
    F: Future<Output = Result<()>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .context("failed to create Netlink runtime")?;
    runtime.block_on(operation)
}

#[cfg(test)]
mod tests {
    use super::parse_prefix;

    #[test]
    fn only_accepts_the_first_version_prefix_contract() {
        assert!(parse_prefix("2001:db8::/64").is_ok());
        assert!(parse_prefix("2001:db8::/63").is_err());
    }
}
