use crate::config::ConnectionPolicy;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::Notify;
use tokio::time::{Duration, timeout};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestAdmission {
    Accepted,
    Draining,
}

pub struct ConnectionLifecycle {
    draining: AtomicBool,
    active: AtomicUsize,
    idle: Notify,
}

impl ConnectionLifecycle {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            draining: AtomicBool::new(false),
            active: AtomicUsize::new(0),
            idle: Notify::new(),
        })
    }
    pub fn begin_request(self: &Arc<Self>) -> Result<RequestGuard, RequestAdmission> {
        if self.draining.load(Ordering::Acquire) {
            return Err(RequestAdmission::Draining);
        }
        self.active.fetch_add(1, Ordering::AcqRel);
        if self.draining.load(Ordering::Acquire) {
            self.release();
            return Err(RequestAdmission::Draining);
        }
        Ok(RequestGuard {
            lifecycle: Arc::clone(self),
        })
    }
    pub fn mark_draining(&self) {
        self.draining.store(true, Ordering::Release);
        if self.active.load(Ordering::Acquire) == 0 {
            self.idle.notify_waiters();
        }
    }
    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::Acquire)
    }
    pub async fn transition(&self, policy: ConnectionPolicy, drain_timeout: Duration) -> bool {
        match policy {
            ConnectionPolicy::Close => {
                self.mark_draining();
                true
            }
            ConnectionPolicy::NextConnection | ConnectionPolicy::Drain => {
                self.mark_draining();
                timeout(drain_timeout, self.wait_idle()).await.is_ok()
            }
        }
    }
    async fn wait_idle(&self) {
        while self.active.load(Ordering::Acquire) != 0 {
            self.idle.notified().await;
        }
    }
    fn release(&self) {
        if self.active.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.idle.notify_waiters();
        }
    }
}

pub struct RequestGuard {
    lifecycle: Arc<ConnectionLifecycle>,
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.lifecycle.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drain_rejects_new_requests_and_waits_for_active_ones() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let lifecycle = ConnectionLifecycle::new();
            let guard = lifecycle.begin_request().unwrap();
            lifecycle.mark_draining();
            assert!(lifecycle.begin_request().is_err());
            let pending = lifecycle.transition(ConnectionPolicy::Drain, Duration::from_millis(50));
            drop(guard);
            assert!(pending.await);
        });
    }
    #[test]
    fn close_does_not_wait_for_active_requests() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let lifecycle = ConnectionLifecycle::new();
            let _guard = lifecycle.begin_request().unwrap();
            assert!(
                lifecycle
                    .transition(ConnectionPolicy::Close, Duration::from_secs(1))
                    .await
            );
        });
    }
}
