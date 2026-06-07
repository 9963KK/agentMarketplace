use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::types::{AgentId, HeartbeatConfig, HeartbeatEvent, PingInfo, PingOutcome};

#[derive(Debug)]
pub struct HeartbeatCore {
    pings: HashMap<AgentId, PingInfo>,
    config: HeartbeatConfig,
}

impl Default for HeartbeatCore {
    fn default() -> Self {
        Self::new(HeartbeatConfig::default())
    }
}

impl HeartbeatCore {
    pub fn new(config: HeartbeatConfig) -> Self {
        Self {
            pings: HashMap::new(),
            config,
        }
    }

    pub fn config(&self) -> HeartbeatConfig {
        self.config
    }

    pub fn ping(&mut self, agent_id: AgentId, busy: bool, now: Instant) -> PingOutcome {
        match self.pings.get_mut(&agent_id) {
            Some(info) => {
                let outcome = if info.timed_out {
                    PingOutcome::RecoveredFromTimeout
                } else {
                    PingOutcome::Updated
                };

                info.last_ping = now;
                info.busy = busy;
                info.timed_out = false;

                outcome
            }
            None => {
                self.pings.insert(agent_id, PingInfo::new(now, busy));
                PingOutcome::FirstSeen
            }
        }
    }

    pub fn is_alive(&self, agent_id: &AgentId, now: Instant) -> bool {
        let Some(info) = self.pings.get(agent_id) else {
            return false;
        };

        if info.timed_out {
            return false;
        }

        elapsed_since(info.last_ping, now) <= self.config.timeout_for(info.busy)
    }

    pub fn scan(&mut self, now: Instant) -> Vec<HeartbeatEvent> {
        let config = self.config;
        let mut events = Vec::new();

        for (agent_id, info) in &mut self.pings {
            if info.timed_out {
                continue;
            }

            let elapsed = elapsed_since(info.last_ping, now);
            if elapsed > config.timeout_for(info.busy) {
                info.timed_out = true;
                events.push(HeartbeatEvent::AgentTimedOut {
                    agent_id: agent_id.clone(),
                });
            }
        }

        events
    }

    pub fn forget(&mut self, agent_id: &AgentId) -> bool {
        self.pings.remove(agent_id).is_some()
    }

    pub fn ping_info(&self, agent_id: &AgentId) -> Option<&PingInfo> {
        self.pings.get(agent_id)
    }

    pub fn len(&self) -> usize {
        self.pings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pings.is_empty()
    }
}

fn elapsed_since(start: Instant, now: Instant) -> Duration {
    now.checked_duration_since(start).unwrap_or(Duration::ZERO)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::super::types::{AgentIdError, HeartbeatConfigError};
    use super::*;

    fn agent(id: &str) -> AgentId {
        AgentId::from(id)
    }

    fn core() -> HeartbeatCore {
        HeartbeatCore::default()
    }

    #[test]
    fn first_ping_registers_agent() {
        let mut core = core();
        let now = Instant::now();

        let outcome = core.ping(agent("agent-1"), false, now);

        assert_eq!(outcome, PingOutcome::FirstSeen);
        assert_eq!(core.len(), 1);
        assert!(core.is_alive(&agent("agent-1"), now));
    }

    #[test]
    fn later_ping_updates_timestamp_and_busy_state() {
        let mut core = core();
        let first = Instant::now();
        let second = first + Duration::from_secs(3);

        core.ping(agent("agent-1"), false, first);
        let outcome = core.ping(agent("agent-1"), true, second);

        let info = core.ping_info(&agent("agent-1")).unwrap();
        assert_eq!(outcome, PingOutcome::Updated);
        assert_eq!(info.last_ping, second);
        assert!(info.busy);
        assert!(!info.timed_out);
    }

    #[test]
    fn idle_agent_is_alive_at_timeout_boundary() {
        let mut core = core();
        let now = Instant::now();
        let agent_id = agent("agent-1");

        core.ping(agent_id.clone(), false, now);

        assert!(core.is_alive(&agent_id, now + Duration::from_secs(45)));
        assert!(core.scan(now + Duration::from_secs(45)).is_empty());
    }

    #[test]
    fn idle_agent_times_out_after_idle_threshold() {
        let mut core = core();
        let now = Instant::now();
        let agent_id = agent("agent-1");

        core.ping(agent_id.clone(), false, now);
        let events = core.scan(now + Duration::from_secs(46));

        assert_eq!(
            events,
            vec![HeartbeatEvent::AgentTimedOut {
                agent_id: agent_id.clone()
            }]
        );
        assert!(!core.is_alive(&agent_id, now + Duration::from_secs(46)));
    }

    #[test]
    fn busy_agent_uses_short_timeout() {
        let mut core = core();
        let now = Instant::now();
        let agent_id = agent("agent-1");

        core.ping(agent_id.clone(), true, now);

        assert!(core.scan(now + Duration::from_secs(15)).is_empty());
        assert_eq!(
            core.scan(now + Duration::from_secs(16)),
            vec![HeartbeatEvent::AgentTimedOut { agent_id }]
        );
    }

    #[test]
    fn timed_out_agent_does_not_emit_duplicate_events() {
        let mut core = core();
        let now = Instant::now();
        let agent_id = agent("agent-1");

        core.ping(agent_id.clone(), false, now);

        assert_eq!(
            core.scan(now + Duration::from_secs(46)),
            vec![HeartbeatEvent::AgentTimedOut { agent_id }]
        );
        assert!(core.scan(now + Duration::from_secs(60)).is_empty());
    }

    #[test]
    fn ping_recovers_timed_out_agent() {
        let mut core = core();
        let now = Instant::now();
        let agent_id = agent("agent-1");

        core.ping(agent_id.clone(), false, now);
        core.scan(now + Duration::from_secs(46));

        let recovered_at = now + Duration::from_secs(47);
        let outcome = core.ping(agent_id.clone(), false, recovered_at);

        assert_eq!(outcome, PingOutcome::RecoveredFromTimeout);
        assert!(core.is_alive(&agent_id, recovered_at));
        assert!(!core.ping_info(&agent_id).unwrap().timed_out);
    }

    #[test]
    fn forget_removes_agent_from_tracking() {
        let mut core = core();
        let now = Instant::now();
        let agent_id = agent("agent-1");

        core.ping(agent_id.clone(), true, now);

        assert!(core.forget(&agent_id));
        assert!(!core.is_alive(&agent_id, now));
        assert!(core.scan(now + Duration::from_secs(100)).is_empty());
        assert!(!core.forget(&agent_id));
    }

    #[test]
    fn custom_config_is_used_for_thresholds() {
        let mut core = HeartbeatCore::new(HeartbeatConfig {
            scan_interval: Duration::from_secs(1),
            idle_timeout: Duration::from_secs(10),
            busy_timeout: Duration::from_secs(2),
        });
        let now = Instant::now();
        let agent_id = agent("agent-1");

        core.ping(agent_id.clone(), true, now);

        assert!(core.scan(now + Duration::from_secs(2)).is_empty());
        assert_eq!(
            core.scan(now + Duration::from_secs(3)),
            vec![HeartbeatEvent::AgentTimedOut { agent_id }]
        );
    }

    #[test]
    fn scan_reports_multiple_timed_out_agents() {
        let mut core = core();
        let now = Instant::now();
        let agent_1 = agent("agent-1");
        let agent_2 = agent("agent-2");
        let agent_3 = agent("agent-3");

        core.ping(agent_1.clone(), false, now);
        core.ping(agent_2.clone(), true, now);
        core.ping(agent_3.clone(), false, now + Duration::from_secs(40));

        let events = core.scan(now + Duration::from_secs(46));

        assert_eq!(events.len(), 2);
        assert!(events.contains(&HeartbeatEvent::AgentTimedOut { agent_id: agent_1 }));
        assert!(events.contains(&HeartbeatEvent::AgentTimedOut { agent_id: agent_2 }));
        assert!(core.is_alive(&agent_3, now + Duration::from_secs(46)));
    }

    #[test]
    fn unknown_agent_is_not_alive() {
        let core = core();

        assert!(!core.is_alive(&agent("missing"), Instant::now()));
    }

    #[test]
    fn invalid_config_is_rejected() {
        assert_eq!(
            HeartbeatConfig {
                scan_interval: Duration::ZERO,
                idle_timeout: Duration::from_secs(45),
                busy_timeout: Duration::from_secs(15),
            }
            .validate()
            .unwrap_err(),
            HeartbeatConfigError::ZeroScanInterval
        );
        assert_eq!(
            HeartbeatConfig {
                scan_interval: Duration::from_secs(5),
                idle_timeout: Duration::ZERO,
                busy_timeout: Duration::from_secs(15),
            }
            .validate()
            .unwrap_err(),
            HeartbeatConfigError::ZeroIdleTimeout
        );
        assert_eq!(
            HeartbeatConfig {
                scan_interval: Duration::from_secs(5),
                idle_timeout: Duration::from_secs(45),
                busy_timeout: Duration::ZERO,
            }
            .validate()
            .unwrap_err(),
            HeartbeatConfigError::ZeroBusyTimeout
        );
    }

    #[test]
    fn blank_agent_id_is_rejected() {
        assert_eq!(AgentId::try_new("  ").unwrap_err(), AgentIdError::Empty);
    }
}
