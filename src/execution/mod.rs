mod queue;
mod rotation;

pub use queue::WorkQueue;
pub use rotation::RotationState;

use crate::config::AddressMode;
use anyhow::{Context, Result};
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
    pool: Arc<Mutex<Option<Ipv6Addr>>>,
    used: Arc<Mutex<HashSet<u64>>>,
}
impl AddressAllocator {
    pub fn rotate(&self, worker: usize) -> Result<()> {
        let mut workers = self
            .workers
            .lock()
            .expect("address allocator mutex poisoned");
        if worker >= workers.len() {
            anyhow::bail!("worker index {worker} is outside configured worker count");
        }
        match self.mode {
            AddressMode::Worker => workers[worker] = None,
            AddressMode::Pool => {
                *self.pool.lock().expect("address allocator mutex poisoned") = None;
            }
            AddressMode::Request => {}
        }
        Ok(())
    }
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
            pool: Arc::new(Mutex::new(None)),
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
        if matches!(self.mode, AddressMode::Pool) {
            let mut pool = self.pool.lock().expect("address allocator mutex poisoned");
            if let Some(address) = *pool {
                return Ok(address);
            }
            let address = self.allocate()?;
            *pool = Some(address);
            return Ok(address);
        }
        let address = self.allocate()?;
        if matches!(self.mode, AddressMode::Worker) {
            workers[worker] = Some(address);
        }
        Ok(address)
    }
    fn allocate(&self) -> Result<Ipv6Addr> {
        let iid = self.unique_iid()?;
        Ok(Ipv6Addr::from((u128::from(self.prefix) << 64) | iid))
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

pub struct WorkerCoordinator {
    allocator: AddressAllocator,
    states: Arc<Mutex<Vec<RotationState>>>,
    scope: crate::config::RotationScope,
}

impl WorkerCoordinator {
    pub fn new(
        prefix: &str,
        mode: AddressMode,
        workers: u16,
        scope: crate::config::RotationScope,
        every_requests: Option<u64>,
        every_ms: Option<u64>,
    ) -> Result<Self> {
        let allocator = AddressAllocator::new(prefix, mode, workers)?;
        let mut states = Vec::with_capacity(workers as usize);
        for _ in 0..workers {
            states.push(
                RotationState::new(every_requests, every_ms)
                    .map_err(|error| anyhow::anyhow!(error))?,
            );
        }
        Ok(Self {
            allocator,
            states: Arc::new(Mutex::new(states)),
            scope,
        })
    }

    pub fn next_address(&self, worker: usize) -> Result<Ipv6Addr> {
        let mut states = self.states.lock().expect("rotation state mutex poisoned");
        if worker >= states.len() {
            anyhow::bail!("worker index {worker} is outside configured worker count");
        }
        let rotate = match self.scope {
            crate::config::RotationScope::Worker => states[worker].record_request(),
            crate::config::RotationScope::Global => {
                let mut rotate = false;
                for state in &mut *states {
                    rotate |= state.record_request();
                }
                rotate
            }
        };
        if rotate {
            match self.scope {
                crate::config::RotationScope::Worker => self.allocator.rotate(worker)?,
                crate::config::RotationScope::Global => {
                    for index in 0..states.len() {
                        self.allocator.rotate(index)?;
                    }
                }
            }
        }
        self.allocator.next(worker)
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
    #[test]
    fn pool_mode_reuses_one_address_across_workers() {
        let pool = AddressAllocator::new("2001:db8::/64", AddressMode::Pool, 2).unwrap();
        assert_eq!(pool.next(0).unwrap(), pool.next(1).unwrap());
    }
    #[test]
    fn worker_rotation_assigns_a_new_address() {
        let allocator = AddressAllocator::new("2001:db8::/64", AddressMode::Worker, 1).unwrap();
        let first = allocator.next(0).unwrap();
        allocator.rotate(0).unwrap();
        assert_ne!(first, allocator.next(0).unwrap());
    }
    #[test]
    fn coordinator_rotates_worker_scope_by_count() {
        let coordinator = WorkerCoordinator::new(
            "2001:db8::/64",
            AddressMode::Worker,
            1,
            crate::config::RotationScope::Worker,
            Some(1),
            None,
        )
        .unwrap();
        let first = coordinator.next_address(0).unwrap();
        let second = coordinator.next_address(0).unwrap();
        assert_ne!(first, second);
    }
}
