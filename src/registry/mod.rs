mod core;
mod service;
mod types;

pub use core::RegistryCore;
pub use service::{RegistryCommand, RegistryHandle, RegistryService, RegistryServiceError};
pub use types::{
    AgentCandidate, AgentIdentity, AgentInfo, AgentLifecycle, Capability, CapabilityName,
    CapabilityUpdateOutcome, DiscoveryQuery, LoadInfo, RegisterOutcome, RegistryError,
};
