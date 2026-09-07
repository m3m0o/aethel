mod rotation;

use crate::config::AddressMode;
use anyhow::{Context, Result};
pub use rotation::RotationState;
use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::net::Ipv6Addr;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct AddressAllocator {
    prefix: u64,
    mode: AddressMode,
    workers: Arc<Mutex<Vec<Option<Ipv6Addr>>>>,
    used: Arc<Mutex<HashSet<u64>>>,
}
impl AddressAllocator {
    pub fn new(prefix: &str, mode: AddressMode, worker_count: u16) -> Result<Self> {
        let (address, length) = prefix
            .split_once('/')
            .ok_or_else(|| anyhow::anyhow!("invalid IPv6 prefix: {prefix}"))?;
        let address = address
            .parse::<Ipv6Addr>()
            .with_context(|| format!("invalid IPv6 prefix address: {address}"))?;
        let length = length
            .parse::<u8>()
            .with_context(|| format!("invalid IPv6 prefix length: {length}"))?;
        if length != 64 {
            anyhow::bail!("address allocation requires an IPv6 /64 prefix, got /{length}");
        }
        if worker_count == 0 {
            anyhow::bail!("address allocation requires at least one worker");
        }
        Ok(Self {
            prefix: u128::from(address) as u64,
            mode,
            workers: Arc::new(Mutex::new(vec![None; worker_count as usize])),
            used: Arc::new(Mutex::new(HashSet::new())),
        })
    }
    pub fn next(&self, worker: usize) -> Result<Ipv6Addr> {
        let mut workers = self
            .workers
            .lock()
            .expect("address allocator mutex poisoned");
        if worker >= workers.len() {
            anyhow::bail!("worker index {worker} is outside configured worker count");
        }
        if matches!(self.mode, AddressMode::Worker) {
            if let Some(address) = workers[worker] {
                return Ok(address);
            }
        }
        let iid = self.unique_iid()?;
        let address = Ipv6Addr::from((u128::from(self.prefix) << 64) | iid);
        if matches!(self.mode, AddressMode::Worker) {
            workers[worker] = Some(address);
        }
        Ok(address)
    }
    fn unique_iid(&self) -> Result<u64> {
        let mut used = self.used.lock().expect("address allocator mutex poisoned");
        for _ in 0..128 {
            let iid = random_u64()?;
            if iid != 0 && iid != u64::MAX && used.insert(iid) {
                return Ok(iid);
            }
        }
        anyhow::bail!("failed to allocate a unique IPv6 IID")
    }
}
fn random_u64() -> Result<u64> {
    let mut bytes = [0u8; 8];
    File::open("/dev/urandom")
        .context("failed to open Linux system random source")?
        .read_exact(&mut bytes)
        .context("failed to read Linux system random source")?;
    Ok(u64::from_ne_bytes(bytes))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_and_request_modes() {
        let w = AddressAllocator::new("2001:db8:1:2::/64", AddressMode::Worker, 2).unwrap();
        assert_eq!(w.next(0).unwrap(), w.next(0).unwrap());
        assert_ne!(w.next(0).unwrap(), w.next(1).unwrap());
        let r = AddressAllocator::new("2001:db8:1:2::/64", AddressMode::Request, 1).unwrap();
        assert_ne!(r.next(0).unwrap(), r.next(0).unwrap());
    }
    #[test]
    fn rejects_invalid_prefix_and_worker() {
        assert!(AddressAllocator::new("2001:db8::/63", AddressMode::Pool, 1).is_err());
        let a = AddressAllocator::new("2001:db8::/64", AddressMode::Pool, 1).unwrap();
        assert!(a.next(1).is_err());
    }
}
