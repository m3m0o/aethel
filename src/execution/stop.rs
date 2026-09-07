use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct StopPolicy {
    pub max_requests: Option<u64>,
    pub max_duration: Option<Duration>,
    pub max_errors: Option<u64>,
    pub max_error_ratio: Option<f64>,
    pub stop_on_match: bool,
}

#[derive(Debug, Default)]
pub struct StopState {
    started: Instant,
    requests: u64,
    errors: u64,
    matched: bool,
}
impl StopState {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            requests: 0,
            errors: 0,
            matched: false,
        }
    }
    pub fn record(&mut self, error: bool, matched: bool, policy: StopPolicy) -> bool {
        self.requests = self.requests.saturating_add(1);
        self.errors = self.errors.saturating_add(u64::from(error));
        self.matched |= matched;
        policy
            .max_requests
            .is_some_and(|limit| self.requests >= limit)
            || policy
                .max_duration
                .is_some_and(|limit| self.started.elapsed() >= limit)
            || policy.max_errors.is_some_and(|limit| self.errors >= limit)
            || policy.max_error_ratio.is_some_and(|limit| {
                self.requests > 0 && self.errors as f64 / self.requests as f64 >= limit
            })
            || (policy.stop_on_match && matched)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stop_conditions_use_or_precedence() {
        let policy = StopPolicy {
            max_requests: Some(2),
            max_duration: None,
            max_errors: Some(5),
            max_error_ratio: None,
            stop_on_match: true,
        };
        let mut state = StopState::new();
        assert!(state.record(false, true, policy));
    }
    #[test]
    fn error_ratio_stops_after_observed_error() {
        let policy = StopPolicy {
            max_requests: None,
            max_duration: None,
            max_errors: None,
            max_error_ratio: Some(0.5),
            stop_on_match: false,
        };
        let mut state = StopState::new();
        assert!(!state.record(false, false, policy));
        assert!(state.record(true, false, policy));
    }
}
