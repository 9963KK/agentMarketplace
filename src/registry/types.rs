use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::artifact::MediaProfileId;
use crate::heartbeat::AgentId;

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct CapabilityName(String);

impl CapabilityName {
    pub fn new(value: impl Into<String>) -> Self {
        Self::try_new(value).expect("capability name must not be empty")
    }

    pub fn try_new(value: impl Into<String>) -> Result<Self, RegistryError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(RegistryError::EmptyCapabilityName);
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for CapabilityName {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for CapabilityName {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for CapabilityName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentIdentity {
    pub agent_id: AgentId,
    pub name: Option<String>,
    pub endpoint: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

impl AgentIdentity {
    pub fn new(agent_id: AgentId) -> Self {
        Self {
            agent_id,
            name: None,
            endpoint: None,
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilityContract {
    pub input_profiles: Vec<MediaProfileId>,
    pub output_profiles: Vec<MediaProfileId>,
}

impl CapabilityContract {
    pub fn new(input_profiles: Vec<MediaProfileId>, output_profiles: Vec<MediaProfileId>) -> Self {
        Self {
            input_profiles,
            output_profiles,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Capability {
    pub name: CapabilityName,
    pub max_concurrency: u32,
    pub contract: Option<CapabilityContract>,
}

impl Capability {
    pub fn new(name: impl Into<CapabilityName>, max_concurrency: u32) -> Self {
        Self {
            name: name.into(),
            max_concurrency,
            contract: None,
        }
    }

    pub fn with_contract(mut self, contract: CapabilityContract) -> Self {
        self.contract = Some(contract);
        self
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LoadInfo {
    pub current: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AgentLifecycle {
    Registered,
    Deregistered,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentInfo {
    pub identity: AgentIdentity,
    pub capabilities: BTreeMap<CapabilityName, Capability>,
    pub lifecycle: AgentLifecycle,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DiscoveryQuery {
    pub capability: CapabilityName,
    pub include_busy: bool,
    pub limit: Option<usize>,
}

impl DiscoveryQuery {
    pub fn new(capability: impl Into<CapabilityName>) -> Self {
        Self {
            capability: capability.into(),
            include_busy: false,
            limit: None,
        }
    }

    pub fn include_busy(mut self, include_busy: bool) -> Self {
        self.include_busy = include_busy;
        self
    }

    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentCandidate {
    pub agent_id: AgentId,
    pub name: Option<String>,
    pub endpoint: Option<String>,
    pub capability: Capability,
    pub current_load: u32,
    pub max_concurrency: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RegisterOutcome {
    Registered,
    Updated,
    ReRegistered,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum CapabilityUpdateOutcome {
    Declared,
    Replaced,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryError {
    EmptyCapabilityName,
    EmptyCapabilityList,
    DuplicateCapability(CapabilityName),
    ZeroMaxConcurrency(CapabilityName),
    DuplicateMediaProfile(MediaProfileId),
    UnsupportedMediaProfile(MediaProfileId),
    AgentNotFound(AgentId),
    AgentDeregistered(AgentId),
    LoadExceedsCapacity {
        agent_id: AgentId,
        current: u32,
        max: u32,
    },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::EmptyCapabilityName => f.write_str("capability name must not be empty"),
            RegistryError::EmptyCapabilityList => f.write_str("capability list must not be empty"),
            RegistryError::DuplicateCapability(name) => {
                write!(f, "duplicate capability: {name}")
            }
            RegistryError::ZeroMaxConcurrency(name) => {
                write!(
                    f,
                    "capability max concurrency must be greater than zero: {name}"
                )
            }
            RegistryError::DuplicateMediaProfile(profile) => {
                write!(f, "duplicate capability media profile: {profile}")
            }
            RegistryError::UnsupportedMediaProfile(profile) => {
                write!(f, "unsupported capability media profile: {profile}")
            }
            RegistryError::AgentNotFound(agent_id) => {
                write!(f, "agent not found: {agent_id}")
            }
            RegistryError::AgentDeregistered(agent_id) => {
                write!(f, "agent is deregistered: {agent_id}")
            }
            RegistryError::LoadExceedsCapacity {
                agent_id,
                current,
                max,
            } => {
                write!(
                    f,
                    "agent load exceeds capacity: {agent_id}, current={current}, max={max}"
                )
            }
        }
    }
}

impl Error for RegistryError {}
