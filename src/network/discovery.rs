use std::fs;
use std::net::Ipv6Addr;
use std::str::FromStr;

use anyhow::{Context, Result};

use crate::config::{
    NdpBackend, NetworkConfig, NetworkSection, RouteSection, StateSection, SysctlSection,
};

pub fn discover() -> Result<NetworkConfig> {
    #[cfg(target_os = "linux")]
    {
        let interface = default_route_interface()?;
        let address = global_address(&interface)?;
        let prefix = Ipv6Addr::from(u128::from(address) & (!0u128 << 64));
        return Ok(NetworkConfig {
            version: 1,
            network: NetworkSection {
                interface,
                prefix: format!("{prefix}/64"),
                loopback: "lo".to_owned(),
                backend: NdpBackend::Native,
            },
            route: RouteSection::default(),
            sysctl: SysctlSection::default(),
            state: StateSection::default(),
        });
    }

    #[cfg(not(target_os = "linux"))]
    {
        Err(anyhow::anyhow!(
            "automatic network discovery is supported only on Linux"
        ))
    }
}

#[cfg(target_os = "linux")]
fn default_route_interface() -> Result<String> {
    let routes = fs::read_to_string("/proc/net/ipv6_route")
        .context("failed to read IPv6 routes for network discovery")?;
    let interface = routes.lines().find_map(|line| {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        (fields.len() >= 10 && fields[0].chars().all(|value| value == '0') && fields[1] == "00")
            .then(|| fields[9].to_owned())
    });
    interface.ok_or_else(|| anyhow::anyhow!("could not discover the default IPv6 route interface"))
}

#[cfg(target_os = "linux")]
fn global_address(interface: &str) -> Result<Ipv6Addr> {
    let addresses = fs::read_to_string("/proc/net/if_inet6")
        .context("failed to read IPv6 addresses for network discovery")?;
    addresses
        .lines()
        .find_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() != 6 || fields[5] != interface || fields[3] != "00" {
                return None;
            }
            parse_hex_address(fields[0]).ok()
        })
        .ok_or_else(|| anyhow::anyhow!("could not discover a global IPv6 address on {interface}"))
}

#[cfg(target_os = "linux")]
fn parse_hex_address(value: &str) -> Result<Ipv6Addr> {
    if value.len() != 32 {
        anyhow::bail!("invalid IPv6 address length: {value}");
    }
    let mut octets = [0u8; 16];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let chunk = std::str::from_utf8(chunk)?;
        octets[index] = u8::from_str_radix(chunk, 16)
            .with_context(|| format!("invalid IPv6 address: {value}"))?;
    }
    Ok(Ipv6Addr::from(octets))
}

#[cfg(test)]
mod tests {
    use std::net::Ipv6Addr;
    use std::str::FromStr;

    #[test]
    fn derives_a_slash_64_from_a_global_address() {
        let address = Ipv6Addr::from_str("2001:db8:1:2::abcd").unwrap();
        let prefix = Ipv6Addr::from(u128::from(address) & (!0u128 << 64));
        assert_eq!(prefix, Ipv6Addr::from_str("2001:db8:1:2::").unwrap());
    }
}
