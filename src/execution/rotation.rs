use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct RotationState {
    request_count: u64,
    last_rotation: Instant,
    every_requests: Option<u64>,
    every: Option<Duration>,
}

impl RotationState {
    pub fn new(every_requests: Option<u64>, every_ms: Option<u64>) -> Result<Self, String> {
        if every_requests == Some(0) || every_ms == Some(0) {
            return Err("rotation intervals must be greater than zero".into());
        }
        Ok(Self {
            request_count: 0,
            last_rotation: Instant::now(),
            every_requests,
            every: every_ms.map(Duration::from_millis),
        })
    }

    pub fn record_request(&mut self) -> bool {
        self.request_count = self.request_count.saturating_add(1);
        let count_due = self
            .every_requests
            .is_some_and(|limit| self.request_count >= limit);
        let time_due = self
            .every
            .is_some_and(|interval| self.last_rotation.elapsed() >= interval);
        if count_due || time_due {
            self.request_count = 0;
            self.last_rotation = Instant::now();
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RotationState;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn rotates_after_configured_request_count() {
        let mut state = RotationState::new(Some(2), None).unwrap();
        assert!(!state.record_request());
        assert!(state.record_request());
        assert!(!state.record_request());
    }

    #[test]
    fn rotates_after_configured_duration() {
        let mut state = RotationState::new(None, Some(1)).unwrap();
        sleep(Duration::from_millis(2));
        assert!(state.record_request());
        assert!(RotationState::new(Some(0), None).is_err());
    }
}
