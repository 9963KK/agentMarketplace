use std::error::Error;
use std::fmt;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AgentId(String);

impl AgentId {
    pub fn new(value: impl Into<String>) -> Self {
        Self::try_new(value).expect("agent id must not be empty")
    }

    pub fn try_new(value: impl Into<String>) -> Result<Self, AgentIdError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(AgentIdError::Empty);
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentIdError {
    Empty,
}

impl fmt::Display for AgentIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentIdError::Empty => f.write_str("agent id must not be empty"),
        }
    }
}

impl Error for AgentIdError {}

impl From<&str> for AgentId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for AgentId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeartbeatConfig {
    pub scan_interval: Duration,
    pub idle_timeout: Duration,
    pub busy_timeout: Duration,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            scan_interval: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(45),
            busy_timeout: Duration::from_secs(15),
        }
    }
}

impl HeartbeatConfig {
    pub fn validate(&self) -> Result<(), HeartbeatConfigError> {
        if self.scan_interval.is_zero() {
            return Err(HeartbeatConfigError::ZeroScanInterval);
        }
        if self.idle_timeout.is_zero() {
            return Err(HeartbeatConfigError::ZeroIdleTimeout);
        }
        if self.busy_timeout.is_zero() {
            return Err(HeartbeatConfigError::ZeroBusyTimeout);
        }

        Ok(())
    }

    pub fn timeout_for(&self, busy: bool) -> Duration {
        if busy {
            self.busy_timeout
        } else {
            self.idle_timeout
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeartbeatConfigError {
    ZeroScanInterval,
    ZeroIdleTimeout,
    ZeroBusyTimeout,
}

impl fmt::Display for HeartbeatConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HeartbeatConfigError::ZeroScanInterval => {
                f.write_str("heartbeat scan interval must be greater than zero")
            }
            HeartbeatConfigError::ZeroIdleTimeout => {
                f.write_str("heartbeat idle timeout must be greater than zero")
            }
            HeartbeatConfigError::ZeroBusyTimeout => {
                f.write_str("heartbeat busy timeout must be greater than zero")
            }
        }
    }
}

impl Error for HeartbeatConfigError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PingInfo {
    pub last_ping: Instant,
    pub busy: bool,
    pub timed_out: bool,
}

impl PingInfo {
    pub fn new(last_ping: Instant, busy: bool) -> Self {
        Self {
            last_ping,
            busy,
            timed_out: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum HeartbeatEvent {
    AgentTimedOut { agent_id: AgentId },
    AgentRecovered { agent_id: AgentId },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PingOutcome {
    FirstSeen,
    Updated,
    RecoveredFromTimeout,
}
