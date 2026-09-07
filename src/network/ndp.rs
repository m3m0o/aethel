use super::capabilities;
use anyhow::{Context, Result};
use std::io;
use std::net::{Ipv6Addr, SocketAddrV6};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const NEIGHBOR_SOLICITATION: u8 = 135;
const NEIGHBOR_ADVERTISEMENT: u8 = 136;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeighborSolicitation {
    pub target: Ipv6Addr,
}

impl NeighborSolicitation {
    pub fn parse(packet: &[u8]) -> Option<Self> {
        if packet.len() < 24 || packet[0] != NEIGHBOR_SOLICITATION || packet[1] != 0 {
            return None;
        }
        let target = Ipv6Addr::from(<[u8; 16]>::try_from(&packet[8..24]).ok()?);
        Some(Self { target })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeighborAdvertisement {
    pub target: Ipv6Addr,
    pub mac: [u8; 6],
}

impl NeighborAdvertisement {
    pub fn encode(&self, solicited: bool) -> Vec<u8> {
        let mut packet = vec![0u8; 32];
        packet[0] = NEIGHBOR_ADVERTISEMENT;
        packet[4] = if solicited { 0x60 } else { 0x20 };
        packet[8..24].copy_from_slice(&self.target.octets());
        packet[24] = 2;
        packet[25] = 1;
        packet[26..32].copy_from_slice(&self.mac);
        packet
    }
}

pub fn is_in_prefix(address: Ipv6Addr, prefix: Ipv6Addr, prefix_length: u8) -> bool {
    if prefix_length > 128 {
        return false;
    }
    let mask = if prefix_length == 0 {
        0
    } else {
        u128::MAX << (128 - prefix_length)
    };
    u128::from(address) & mask == u128::from(prefix) & mask
}

pub fn icmpv6_checksum(source: Ipv6Addr, destination: Ipv6Addr, payload: &[u8]) -> u16 {
    let mut sum = 0u32;
    for word in source
        .octets()
        .chunks_exact(2)
        .chain(destination.octets().chunks_exact(2))
    {
        sum += u16::from_be_bytes([word[0], word[1]]) as u32;
    }
    sum += ((payload.len() as u32) >> 16) + ((payload.len() as u32) & 0xffff);
    sum += 58;
    for word in payload.chunks(2) {
        sum += u16::from_be_bytes([word[0], *word.get(1).unwrap_or(&0)]) as u32;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

pub struct NativeNdpResponder {
    interface: String,
    interface_index: u32,
    prefix: Ipv6Addr,
    prefix_length: u8,
    mac: [u8; 6],
}

impl NativeNdpResponder {
    pub fn new(interface: &str, prefix: Ipv6Addr, prefix_length: u8) -> Result<Self> {
        let capability = capabilities::inspect()?;
        if !capability.net_raw || !capability.net_admin {
            anyhow::bail!(
                "native NDP requires CAP_NET_RAW and CAP_NET_ADMIN (effective: CAP_NET_RAW={}, CAP_NET_ADMIN={})",
                capability.net_raw,
                capability.net_admin
            );
        }
        let interface_index = super::inspect::interface_index(interface)?;
        let mac = read_mac(interface)?;
        Ok(Self {
            interface: interface.to_owned(),
            interface_index,
            prefix,
            prefix_length,
            mac,
        })
    }

    pub fn interface(&self) -> &str {
        &self.interface
    }

    pub fn interface_index(&self) -> u32 {
        self.interface_index
    }

    pub fn run(&self, stop: &AtomicBool) -> Result<()> {
        let socket = socket2::Socket::new(
            socket2::Domain::IPV6,
            socket2::Type::RAW,
            Some(socket2::Protocol::ICMPV6),
        )
        .context("failed to create ICMPv6 raw socket; CAP_NET_RAW may be missing")?;
        socket
            .bind_device_by_index_v6(Some(
                std::num::NonZeroU32::new(self.interface_index)
                    .ok_or_else(|| anyhow::anyhow!("invalid interface index"))?,
            ))
            .context("failed to bind ICMPv6 socket to interface")?;
        socket
            .set_nonblocking(true)
            .context("failed to configure ICMPv6 socket")?;

        let mut buffer = [std::mem::MaybeUninit::<u8>::uninit(); 2048];
        while !stop.load(Ordering::Relaxed) {
            match socket.recv_from(&mut buffer) {
                Ok((length, peer)) => {
                    // SAFETY: recv_from initializes exactly the returned byte range.
                    let packet =
                        unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), length) };
                    self.handle_packet(packet, &peer, &socket)?;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error).context("failed to receive ICMPv6 packet"),
            }
        }
        Ok(())
    }

    fn handle_packet(
        &self,
        packet: &[u8],
        peer: &socket2::SockAddr,
        socket: &socket2::Socket,
    ) -> Result<()> {
        let Some(request) = NeighborSolicitation::parse(packet) else {
            return Ok(());
        };
        if !self.accepts(request.target) {
            return Ok(());
        }
        let Some(peer) = peer.as_socket_ipv6() else {
            return Ok(());
        };
        if peer.ip().is_unspecified() {
            return Ok(());
        }
        let mut response = NeighborAdvertisement {
            target: request.target,
            mac: self.mac,
        }
        .encode(true);
        let source = request.target;
        let checksum = icmpv6_checksum(source, *peer.ip(), &response);
        response[2..4].copy_from_slice(&checksum.to_be_bytes());
        let destination = socket2::SockAddr::from(SocketAddrV6::new(
            *peer.ip(),
            0,
            peer.flowinfo(),
            self.interface_index,
        ));
        socket
            .send_to(&response, &destination)
            .context("failed to send Neighbor Advertisement")?;
        Ok(())
    }
    pub fn accepts(&self, target: Ipv6Addr) -> bool {
        is_in_prefix(target, self.prefix, self.prefix_length)
    }
}

fn read_mac(interface: &str) -> Result<[u8; 6]> {
    let value = std::fs::read_to_string(format!("/sys/class/net/{interface}/address"))
        .with_context(|| format!("failed to read MAC address for {interface}"))?;
    let bytes = value
        .trim()
        .split(':')
        .map(|part| u8::from_str_radix(part, 16))
        .collect::<Result<Vec<_>, _>>()?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("interface {interface} does not have a six-byte MAC address"))
}

#[cfg(test)]
mod tests {
    use super::{NeighborAdvertisement, NeighborSolicitation, icmpv6_checksum, is_in_prefix};
    use std::net::Ipv6Addr;
    use std::str::FromStr;

    #[test]
    fn parses_neighbor_solicitation_target() {
        let target = Ipv6Addr::from_str("2001:db8:1::42").unwrap();
        let mut packet = vec![135, 0, 0, 0, 0, 0, 0, 0];
        packet.extend_from_slice(&target.octets());
        assert_eq!(NeighborSolicitation::parse(&packet).unwrap().target, target);
    }

    #[test]
    fn rejects_target_outside_prefix() {
        let prefix = Ipv6Addr::from_str("2001:db8:1::").unwrap();
        assert!(is_in_prefix(
            Ipv6Addr::from_str("2001:db8:1::42").unwrap(),
            prefix,
            64
        ));
        assert!(!is_in_prefix(
            Ipv6Addr::from_str("2001:db8:2::42").unwrap(),
            prefix,
            64
        ));
    }

    #[test]
    fn encodes_neighbor_advertisement_option() {
        let advertisement = NeighborAdvertisement {
            target: Ipv6Addr::LOCALHOST,
            mac: [0, 1, 2, 3, 4, 5],
        };
        let packet = advertisement.encode(true);
        assert_eq!(packet[0], 136);
        assert_eq!(packet[4], 0x60);
        assert_eq!(&packet[24..32], &[2, 1, 0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn checksum_is_computed_over_ipv6_pseudo_header() {
        let source = Ipv6Addr::LOCALHOST;
        let destination = Ipv6Addr::UNSPECIFIED;
        let first = icmpv6_checksum(source, destination, &[128, 0, 0, 0]);
        let second = icmpv6_checksum(source, destination, &[128, 0, 0, 0]);
        assert_eq!(first, second);
    }
}
