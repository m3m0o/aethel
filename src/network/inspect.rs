use std::collections::BTreeMap;
use std::fs;
use std::net::Ipv6Addr;

use anyhow::{Context, Result};
use futures_util::TryStreamExt;
use netlink_packet_route::route::{RouteAddress, RouteAttribute, RouteMessage, RouteType};
use rtnetlink::RouteMessageBuilder;

use crate::config::{NdpBackend, NetworkConfig};

use super::capabilities;
use super::snapshot::{BackendState, NetworkSnapshot, RouteState};

pub fn host_summary() -> Result<String> {
    #[cfg(target_os = "linux")]
    {
        let snapshot = inspect_host(None)?;
        Ok(format_snapshot(&snapshot, None))
    }

    #[cfg(not(target_os = "linux"))]
    {
        Err(anyhow::anyhow!(
            "network check without --config is supported only on Linux"
        ))
    }
}

pub fn configured_summary(config: &NetworkConfig) -> Result<String> {
    let snapshot = inspect_host(Some(config))?;
    Ok(format_snapshot(&snapshot, Some(&config.network.prefix)))
}

fn inspect_host(config: Option<&NetworkConfig>) -> Result<NetworkSnapshot> {
    #[cfg(target_os = "linux")]
    {
        let interface = config
            .map(|value| value.network.interface.as_str())
            .unwrap_or("lo");
        let interface_index = interface_index(interface)?;
        let routes = read_routes()?;
        let sysctls = read_sysctls()?;
        let capabilities = capabilities::inspect()?;
        let ndp_backend = config
            .map(|value| match value.network.backend {
                NdpBackend::Native => BackendState::Native,
                NdpBackend::Ndppd => BackendState::Ndppd,
            })
            .unwrap_or(BackendState::Native);

        Ok(NetworkSnapshot {
            interface: interface.to_owned(),
            interface_index,
            routes,
            sysctls,
            capabilities,
            ndp_backend,
            restoration_available: config.is_some(),
        })
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = config;
        Err(anyhow::anyhow!(
            "network inspection is supported only on Linux"
        ))
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn interface_index(interface: &str) -> Result<u32> {
    if interface.is_empty() || interface.contains('/') || interface.contains('\\') {
        anyhow::bail!("invalid network interface name: {interface}");
    }
    let path = format!("/sys/class/net/{interface}/ifindex");
    fs::read_to_string(&path)
        .with_context(|| format!("failed to read interface index for {interface}"))?
        .trim()
        .parse()
        .with_context(|| format!("invalid interface index for {interface}"))
}

#[cfg(target_os = "linux")]
fn read_sysctls() -> Result<BTreeMap<String, String>> {
    let mut sysctls = BTreeMap::new();
    sysctls.insert(
        "net.ipv6.conf.all.forwarding".to_owned(),
        read_sysctl("/proc/sys/net/ipv6/conf/all/forwarding")?,
    );
    sysctls.insert(
        "net.ipv6.ip_nonlocal_bind".to_owned(),
        read_sysctl("/proc/sys/net/ipv6/ip_nonlocal_bind")?,
    );
    Ok(sysctls)
}

#[cfg(target_os = "linux")]
fn read_sysctl(path: &str) -> Result<String> {
    Ok(fs::read_to_string(path)
        .with_context(|| format!("failed to read {path}"))?
        .trim()
        .to_owned())
}

#[cfg(target_os = "linux")]
pub(crate) fn read_routes() -> Result<Vec<RouteState>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .context("failed to create Netlink runtime")?;
    runtime.block_on(async {
        let (connection, handle, _) =
            rtnetlink::new_connection().context("failed to open route Netlink connection")?;
        tokio::spawn(connection);

        let request = handle
            .route()
            .get(RouteMessageBuilder::<Ipv6Addr>::new().build());
        let messages = request
            .execute()
            .try_collect::<Vec<RouteMessage>>()
            .await
            .context("failed to read routes through Netlink")?;
        messages.into_iter().map(route_state).collect()
    })
}

#[cfg(target_os = "linux")]
fn route_state(message: RouteMessage) -> Result<RouteState> {
    let prefix_length = message.header.destination_prefix_length;
    let mut destination = None;
    let mut output_interface = None;
    let mut table = u32::from(message.header.table);

    for attribute in message.attributes {
        match attribute {
            RouteAttribute::Destination(address) => {
                destination = Some(format_route_address(address, prefix_length));
            }
            RouteAttribute::Oif(index) => output_interface = Some(index.to_string()),
            RouteAttribute::Table(value) => table = value,
            _ => {}
        }
    }

    Ok(RouteState {
        destination,
        kind: route_type_name(message.header.kind),
        table,
        output_interface,
    })
}
#[cfg(target_os = "linux")]
fn format_route_address(address: RouteAddress, prefix_length: u8) -> String {
    match address {
        RouteAddress::Inet6(address) => format!("{address}/{prefix_length}"),
        RouteAddress::Inet(address) => format!("{address}/{prefix_length}"),
        other => format!("{other:?}/{prefix_length}"),
    }
}

#[cfg(target_os = "linux")]
fn route_type_name(route_type: RouteType) -> String {
    format!("{route_type:?}").to_lowercase()
}

fn format_snapshot(snapshot: &NetworkSnapshot, configured_prefix: Option<&str>) -> String {
    let mut lines = vec![
        "Network status".to_owned(),
        format!(
            "[+] Interface: {} (index {})",
            snapshot.interface, snapshot.interface_index
        ),
    ];
    if let Some(prefix) = configured_prefix {
        let route = snapshot.routes.iter().find(|route| {
            route.destination.as_deref() == Some(prefix)
                && route.kind == "local"
                && route.output_interface.as_deref() == Some(&snapshot.interface_index.to_string())
        });
        match route {
            Some(_) => lines.push(format!(
                "[+] AnyIP route: {prefix} via {}",
                snapshot.interface
            )),
            None => lines.push(format!(
                "[-] AnyIP route missing: {prefix} via {}",
                snapshot.interface
            )),
        }
    } else {
        lines.push(format!("[+] Routes visible: {}", snapshot.routes.len()));
    }
    let forwarding = snapshot
        .sysctls
        .get("net.ipv6.conf.all.forwarding")
        .map(String::as_str)
        .unwrap_or("unknown");
    lines.push(if forwarding == "1" {
        "[+] IPv6 forwarding enabled".to_owned()
    } else {
        format!("[!] IPv6 forwarding disabled (value: {forwarding})")
    });
    let bind = snapshot
        .sysctls
        .get("net.ipv6.ip_nonlocal_bind")
        .map(String::as_str)
        .unwrap_or("unknown");
    lines.push(if bind == "1" {
        "[+] Non-local IPv6 bind enabled".to_owned()
    } else {
        format!("[!] Non-local IPv6 bind disabled (value: {bind})")
    });
    lines.push(if snapshot.capabilities.net_admin {
        "[+] CAP_NET_ADMIN available".to_owned()
    } else {
        "[-] CAP_NET_ADMIN missing: network setup requires root or this capability".to_owned()
    });
    lines.push(if snapshot.capabilities.net_raw {
        "[+] CAP_NET_RAW available".to_owned()
    } else {
        "[-] CAP_NET_RAW missing: native NDP requires root or this capability".to_owned()
    });
    lines.push(format!("[+] NDP backend: {:?}", snapshot.ndp_backend));
    lines.push(if snapshot.restoration_available {
        "[+] Network restoration snapshot available".to_owned()
    } else {
        "[!] No restoration snapshot: run with a network configuration".to_owned()
    });
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::super::snapshot::RouteState;

    #[test]
    fn route_state_keeps_optional_fields() {
        let route = RouteState {
            destination: None,
            kind: "local".to_owned(),
            table: 255,
            output_interface: Some("1".to_owned()),
        };

        assert_eq!(route.destination, None);
        assert_eq!(route.table, 255);
    }
}
