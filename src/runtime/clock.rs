use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::Timestamp;

pub trait RuntimeClock: Clone + Send + Sync + 'static {
    fn now(&self) -> Timestamp;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SystemRuntimeClock;

impl RuntimeClock for SystemRuntimeClock {
    fn now(&self) -> Timestamp {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64;
        Timestamp(millis)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedRuntimeClock {
    timestamp: Timestamp,
}

impl FixedRuntimeClock {
    pub fn new(timestamp: Timestamp) -> Self {
        Self { timestamp }
    }
}

impl RuntimeClock for FixedRuntimeClock {
    fn now(&self) -> Timestamp {
        self.timestamp
    }
}
