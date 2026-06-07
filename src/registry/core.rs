use std::collections::{BTreeMap, HashMap, HashSet};

use crate::heartbeat::AgentId;

use super::types::{
    AgentCandidate, AgentIdentity, AgentInfo, AgentLifecycle, Capability, CapabilityName,
    CapabilityUpdateOutcome, DiscoveryQuery, LoadInfo, RegisterOutcome, RegistryError,
};

#[derive(Debug, Default)]
pub struct RegistryCore {
    agents: HashMap<AgentId, AgentInfo>,
    alive_agents: HashSet<AgentId>,
    capability_index: HashMap<CapabilityName, HashSet<AgentId>>,
    load: HashMap<AgentId, LoadInfo>,
}

impl RegistryCore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, identity: AgentIdentity) -> Result<RegisterOutcome, RegistryError> {
        let agent_id = identity.agent_id.clone();

        match self.agents.get_mut(&agent_id) {
            Some(info) => {
                let outcome = if info.lifecycle == AgentLifecycle::Deregistered {
                    info.lifecycle = AgentLifecycle::Registered;
                    RegisterOutcome::ReRegistered
                } else {
                    RegisterOutcome::Updated
                };
                info.identity = identity;
                Ok(outcome)
            }
            None => {
                self.agents.insert(
                    agent_id.clone(),
                    AgentInfo {
                        identity,
                        capabilities: BTreeMap::new(),
                        lifecycle: AgentLifecycle::Registered,
                    },
                );
                self.load.insert(agent_id, LoadInfo { current: 0 });
                Ok(RegisterOutcome::Registered)
            }
        }
    }

    pub fn declare_capabilities(
        &mut self,
        agent_id: &AgentId,
        capabilities: Vec<Capability>,
    ) -> Result<CapabilityUpdateOutcome, RegistryError> {
        let existing = self.agent_info(agent_id)?;
        let normalized = normalize_capabilities(capabilities)?;
        let outcome = if existing.capabilities.is_empty() {
            CapabilityUpdateOutcome::Declared
        } else {
            CapabilityUpdateOutcome::Replaced
        };

        let old_names: Vec<CapabilityName> = existing.capabilities.keys().cloned().collect();
        for name in old_names {
            self.remove_from_capability_index(&name, agent_id);
        }

        let info = self.agent_info_mut(agent_id)?;
        info.capabilities = normalized;

        let names: Vec<CapabilityName> = info.capabilities.keys().cloned().collect();
        for name in names {
            self.capability_index
                .entry(name)
                .or_default()
                .insert(agent_id.clone());
        }

        Ok(outcome)
    }

    pub fn deregister(&mut self, agent_id: &AgentId) -> bool {
        let Some(info) = self.agents.get_mut(agent_id) else {
            return false;
        };

        if info.lifecycle == AgentLifecycle::Deregistered {
            return false;
        }

        info.lifecycle = AgentLifecycle::Deregistered;
        self.alive_agents.remove(agent_id);

        let old_names: Vec<CapabilityName> = info.capabilities.keys().cloned().collect();
        for name in old_names {
            self.remove_from_capability_index(&name, agent_id);
        }
        if let Some(info) = self.agents.get_mut(agent_id) {
            info.capabilities.clear();
        }

        true
    }

    pub fn mark_alive(&mut self, agent_id: &AgentId) -> bool {
        let Some(info) = self.agents.get(agent_id) else {
            return false;
        };
        if info.lifecycle == AgentLifecycle::Deregistered {
            return false;
        }

        self.alive_agents.insert(agent_id.clone())
    }

    pub fn mark_timed_out(&mut self, agent_id: &AgentId) -> bool {
        self.alive_agents.remove(agent_id)
    }

    pub fn set_load(&mut self, agent_id: &AgentId, current: u32) -> Result<(), RegistryError> {
        let info = self.agent_info(agent_id)?;
        let max = max_capacity(info);
        if current > max {
            return Err(RegistryError::LoadExceedsCapacity {
                agent_id: agent_id.clone(),
                current,
                max,
            });
        }

        self.load.insert(agent_id.clone(), LoadInfo { current });
        Ok(())
    }

    pub fn discover(&self, query: DiscoveryQuery) -> Vec<AgentCandidate> {
        let Some(agent_ids) = self.capability_index.get(&query.capability) else {
            return Vec::new();
        };

        let mut candidates = Vec::new();
        let mut sorted_ids: Vec<&AgentId> = agent_ids.iter().collect();
        sorted_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));

        for agent_id in sorted_ids {
            let Some(info) = self.agents.get(agent_id) else {
                continue;
            };
            if info.lifecycle == AgentLifecycle::Deregistered {
                continue;
            }
            if !self.alive_agents.contains(agent_id) {
                continue;
            }

            let Some(capability) = info.capabilities.get(&query.capability) else {
                continue;
            };

            let current_load = self
                .load
                .get(agent_id)
                .map(|load| load.current)
                .unwrap_or(0);
            if !query.include_busy && current_load >= capability.max_concurrency {
                continue;
            }

            candidates.push(AgentCandidate {
                agent_id: agent_id.clone(),
                name: info.identity.name.clone(),
                endpoint: info.identity.endpoint.clone(),
                capability: capability.clone(),
                current_load,
                max_concurrency: capability.max_concurrency,
            });

            if query.limit.is_some_and(|limit| candidates.len() >= limit) {
                break;
            }
        }

        candidates
    }

    pub fn get(&self, agent_id: &AgentId) -> Option<&AgentInfo> {
        self.agents.get(agent_id)
    }

    pub fn is_alive(&self, agent_id: &AgentId) -> bool {
        self.alive_agents.contains(agent_id)
    }

    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    fn agent_info(&self, agent_id: &AgentId) -> Result<&AgentInfo, RegistryError> {
        let info = self
            .agents
            .get(agent_id)
            .ok_or_else(|| RegistryError::AgentNotFound(agent_id.clone()))?;
        if info.lifecycle == AgentLifecycle::Deregistered {
            return Err(RegistryError::AgentDeregistered(agent_id.clone()));
        }

        Ok(info)
    }

    fn agent_info_mut(&mut self, agent_id: &AgentId) -> Result<&mut AgentInfo, RegistryError> {
        let info = self
            .agents
            .get_mut(agent_id)
            .ok_or_else(|| RegistryError::AgentNotFound(agent_id.clone()))?;
        if info.lifecycle == AgentLifecycle::Deregistered {
            return Err(RegistryError::AgentDeregistered(agent_id.clone()));
        }

        Ok(info)
    }

    fn remove_from_capability_index(&mut self, name: &CapabilityName, agent_id: &AgentId) {
        let Some(agent_ids) = self.capability_index.get_mut(name) else {
            return;
        };

        agent_ids.remove(agent_id);
        if agent_ids.is_empty() {
            self.capability_index.remove(name);
        }
    }
}

fn normalize_capabilities(
    capabilities: Vec<Capability>,
) -> Result<BTreeMap<CapabilityName, Capability>, RegistryError> {
    if capabilities.is_empty() {
        return Err(RegistryError::EmptyCapabilityList);
    }

    let mut normalized = BTreeMap::new();
    for capability in capabilities {
        if capability.max_concurrency == 0 {
            return Err(RegistryError::ZeroMaxConcurrency(capability.name));
        }

        let name = capability.name.clone();
        if normalized.insert(name.clone(), capability).is_some() {
            return Err(RegistryError::DuplicateCapability(name));
        }
    }

    Ok(normalized)
}

fn max_capacity(info: &AgentInfo) -> u32 {
    info.capabilities
        .values()
        .map(|capability| capability.max_concurrency)
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn agent(id: &str) -> AgentId {
        AgentId::from(id)
    }

    fn identity(id: &str) -> AgentIdentity {
        AgentIdentity {
            agent_id: agent(id),
            name: Some(format!("Agent {id}")),
            endpoint: Some(format!("https://{id}.example.test")),
            metadata: BTreeMap::new(),
        }
    }

    fn capability(name: &str, max_concurrency: u32) -> Capability {
        Capability::new(name, max_concurrency)
    }

    fn query(name: &str) -> DiscoveryQuery {
        DiscoveryQuery::new(name)
    }

    #[test]
    fn register_stores_identity_but_does_not_make_agent_discoverable() {
        let mut registry = RegistryCore::new();

        let outcome = registry.register(identity("agent-1")).unwrap();

        assert_eq!(outcome, RegisterOutcome::Registered);
        assert!(registry.get(&agent("agent-1")).is_some());
        assert!(registry.discover(query("code-review")).is_empty());
    }

    #[test]
    fn declared_capability_still_requires_alive_heartbeat() {
        let mut registry = RegistryCore::new();
        let agent_id = agent("agent-1");

        registry.register(identity("agent-1")).unwrap();
        registry
            .declare_capabilities(&agent_id, vec![capability("code-review", 2)])
            .unwrap();

        assert!(registry.discover(query("code-review")).is_empty());
    }

    #[test]
    fn alive_agent_with_capability_is_discoverable() {
        let mut registry = RegistryCore::new();
        let agent_id = agent("agent-1");

        registry.register(identity("agent-1")).unwrap();
        registry
            .declare_capabilities(&agent_id, vec![capability("code-review", 2)])
            .unwrap();
        assert!(registry.mark_alive(&agent_id));

        let candidates = registry.discover(query("code-review"));
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].agent_id, agent_id);
        assert_eq!(candidates[0].current_load, 0);
        assert_eq!(candidates[0].max_concurrency, 2);
    }

    #[test]
    fn timed_out_agent_is_not_discoverable_but_info_remains() {
        let mut registry = RegistryCore::new();
        let agent_id = agent("agent-1");

        registry.register(identity("agent-1")).unwrap();
        registry
            .declare_capabilities(&agent_id, vec![capability("code-review", 2)])
            .unwrap();
        registry.mark_alive(&agent_id);

        assert!(registry.mark_timed_out(&agent_id));

        assert!(registry.discover(query("code-review")).is_empty());
        assert!(registry.get(&agent_id).is_some());
    }

    #[test]
    fn recovered_agent_becomes_discoverable_again() {
        let mut registry = RegistryCore::new();
        let agent_id = agent("agent-1");

        registry.register(identity("agent-1")).unwrap();
        registry
            .declare_capabilities(&agent_id, vec![capability("code-review", 2)])
            .unwrap();
        registry.mark_alive(&agent_id);
        registry.mark_timed_out(&agent_id);

        assert!(registry.mark_alive(&agent_id));

        assert_eq!(registry.discover(query("code-review")).len(), 1);
    }

    #[test]
    fn deregistered_agent_cannot_be_restored_by_mark_alive() {
        let mut registry = RegistryCore::new();
        let agent_id = agent("agent-1");

        registry.register(identity("agent-1")).unwrap();
        registry
            .declare_capabilities(&agent_id, vec![capability("code-review", 2)])
            .unwrap();
        registry.mark_alive(&agent_id);

        assert!(registry.deregister(&agent_id));
        assert!(!registry.mark_alive(&agent_id));

        assert!(registry.discover(query("code-review")).is_empty());
        assert_eq!(
            registry.get(&agent_id).unwrap().lifecycle,
            AgentLifecycle::Deregistered
        );
    }

    #[test]
    fn re_register_replaces_identity_without_restoring_capabilities() {
        let mut registry = RegistryCore::new();
        let agent_id = agent("agent-1");
        let mut updated = identity("agent-1");
        updated.name = Some("Updated".to_string());

        registry.register(identity("agent-1")).unwrap();
        registry
            .declare_capabilities(&agent_id, vec![capability("code-review", 2)])
            .unwrap();
        registry.mark_alive(&agent_id);
        registry.deregister(&agent_id);

        let outcome = registry.register(updated).unwrap();

        assert_eq!(outcome, RegisterOutcome::ReRegistered);
        assert_eq!(
            registry.get(&agent_id).unwrap().identity.name.as_deref(),
            Some("Updated")
        );
        assert!(registry.get(&agent_id).unwrap().capabilities.is_empty());
        assert!(registry.discover(query("code-review")).is_empty());
    }

    #[test]
    fn repeated_register_only_replaces_identity() {
        let mut registry = RegistryCore::new();
        let agent_id = agent("agent-1");
        let mut updated = identity("agent-1");
        updated.name = Some("Updated".to_string());

        registry.register(identity("agent-1")).unwrap();
        registry
            .declare_capabilities(&agent_id, vec![capability("code-review", 2)])
            .unwrap();
        registry.mark_alive(&agent_id);

        let outcome = registry.register(updated).unwrap();

        assert_eq!(outcome, RegisterOutcome::Updated);
        assert_eq!(
            registry.get(&agent_id).unwrap().identity.name.as_deref(),
            Some("Updated")
        );
        assert_eq!(registry.discover(query("code-review")).len(), 1);
    }

    #[test]
    fn capability_declaration_replaces_old_index() {
        let mut registry = RegistryCore::new();
        let agent_id = agent("agent-1");

        registry.register(identity("agent-1")).unwrap();
        registry
            .declare_capabilities(&agent_id, vec![capability("code-review", 2)])
            .unwrap();
        registry.mark_alive(&agent_id);
        let outcome = registry
            .declare_capabilities(&agent_id, vec![capability("rust", 3)])
            .unwrap();

        assert_eq!(outcome, CapabilityUpdateOutcome::Replaced);
        assert!(registry.discover(query("code-review")).is_empty());
        assert_eq!(registry.discover(query("rust")).len(), 1);
    }

    #[test]
    fn invalid_capability_declaration_is_rejected_without_partial_update() {
        let mut registry = RegistryCore::new();
        let agent_id = agent("agent-1");

        registry.register(identity("agent-1")).unwrap();
        registry
            .declare_capabilities(&agent_id, vec![capability("code-review", 2)])
            .unwrap();
        registry.mark_alive(&agent_id);

        assert_eq!(
            registry
                .declare_capabilities(&agent_id, vec![capability("rust", 0)])
                .unwrap_err(),
            RegistryError::ZeroMaxConcurrency(CapabilityName::from("rust"))
        );
        assert_eq!(registry.discover(query("code-review")).len(), 1);
        assert!(registry.discover(query("rust")).is_empty());
    }

    #[test]
    fn empty_and_duplicate_capabilities_are_rejected() {
        let mut registry = RegistryCore::new();
        let agent_id = agent("agent-1");

        registry.register(identity("agent-1")).unwrap();

        assert_eq!(
            registry
                .declare_capabilities(&agent_id, Vec::new())
                .unwrap_err(),
            RegistryError::EmptyCapabilityList
        );
        assert_eq!(
            registry
                .declare_capabilities(
                    &agent_id,
                    vec![capability("rust", 1), capability("rust", 2)]
                )
                .unwrap_err(),
            RegistryError::DuplicateCapability(CapabilityName::from("rust"))
        );
    }

    #[test]
    fn full_agent_is_filtered_unless_include_busy_is_set() {
        let mut registry = RegistryCore::new();
        let agent_id = agent("agent-1");

        registry.register(identity("agent-1")).unwrap();
        registry
            .declare_capabilities(&agent_id, vec![capability("code-review", 2)])
            .unwrap();
        registry.mark_alive(&agent_id);
        registry.set_load(&agent_id, 2).unwrap();

        assert!(registry.discover(query("code-review")).is_empty());
        assert_eq!(
            registry
                .discover(DiscoveryQuery::new("code-review").include_busy(true))
                .len(),
            1
        );
    }

    #[test]
    fn load_cannot_exceed_max_capacity() {
        let mut registry = RegistryCore::new();
        let agent_id = agent("agent-1");

        registry.register(identity("agent-1")).unwrap();
        registry
            .declare_capabilities(&agent_id, vec![capability("code-review", 2)])
            .unwrap();

        assert_eq!(
            registry.set_load(&agent_id, 3).unwrap_err(),
            RegistryError::LoadExceedsCapacity {
                agent_id,
                current: 3,
                max: 2
            }
        );
    }

    #[test]
    fn discover_uses_limit_and_deterministic_agent_order() {
        let mut registry = RegistryCore::new();

        for id in ["agent-b", "agent-a", "agent-c"] {
            let agent_id = agent(id);
            registry.register(identity(id)).unwrap();
            registry
                .declare_capabilities(&agent_id, vec![capability("rust", 1)])
                .unwrap();
            registry.mark_alive(&agent_id);
        }

        let candidates = registry.discover(DiscoveryQuery::new("rust").limit(2));

        assert_eq!(
            candidates
                .into_iter()
                .map(|candidate| candidate.agent_id)
                .collect::<Vec<_>>(),
            vec![agent("agent-a"), agent("agent-b")]
        );
    }
}
