use std::future::Future;
use std::net::Ipv6Addr;
use std::str::FromStr;

use crate::config::NetworkConfig;
use anyhow::{Context, Result};
use netlink_packet_route::route::{RouteScope, RouteType};
use rtnetlink::RouteMessageBuilder;

use super::inspect::{interface_index, read_routes};

pub fn setup(config: &NetworkConfig) -> Result<String> {
    let (prefix, prefix_length) = parse_prefix(&config.network.prefix)?;
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
            add_route(prefix, prefix_length, loopback_index)?;
            Ok(format!("AnyIP route created: {}", config.network.prefix))
        }
    }
}

pub fn cleanup(config: &NetworkConfig) -> Result<String> {
    let (prefix, prefix_length) = parse_prefix(&config.network.prefix)?;
    let loopback_index = interface_index(&config.network.loopback)?;
    let routes = read_routes()?;
    match find_route(&routes, prefix, prefix_length, loopback_index) {
        Some(true) => {
            delete_route(prefix, prefix_length, loopback_index)?;
            Ok(format!("AnyIP route removed: {}", config.network.prefix))
        }
        Some(false) => anyhow::bail!(
            "refusing to remove incompatible route for {}",
            config.network.prefix
        ),
        None => Ok(format!("AnyIP route absent: {}", config.network.prefix)),
    }
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
    F: std::future::Future<Output = Result<()>>,
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
