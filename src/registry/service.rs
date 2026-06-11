use std::error::Error;
use std::fmt;

use tokio::sync::{mpsc, oneshot};

use crate::heartbeat::{AgentId, HeartbeatEvent};

use super::RegistryCore;
use super::types::{
    AgentCandidate, AgentIdentity, AgentListing, Capability, CapabilityUpdateOutcome,
    DiscoveryQuery, ListAgentsQuery, RegisterOutcome, RegistryError,
};

const DEFAULT_COMMAND_BUFFER: usize = 128;

#[derive(Debug)]
pub enum RegistryCommand {
    Register {
        identity: AgentIdentity,
        reply: oneshot::Sender<Result<RegisterOutcome, RegistryError>>,
    },
    DeclareCapabilities {
        agent_id: AgentId,
        capabilities: Vec<Capability>,
        reply: oneshot::Sender<Result<CapabilityUpdateOutcome, RegistryError>>,
    },
    Deregister {
        agent_id: AgentId,
        reply: oneshot::Sender<bool>,
    },
    MarkAlive {
        agent_id: AgentId,
    },
    MarkTimedOut {
        agent_id: AgentId,
    },
    ApplyHeartbeatEvent {
        event: HeartbeatEvent,
    },
    SetLoad {
        agent_id: AgentId,
        current: u32,
        reply: oneshot::Sender<Result<(), RegistryError>>,
    },
    Discover {
        query: DiscoveryQuery,
        reply: oneshot::Sender<Vec<AgentCandidate>>,
    },
    ListAgents {
        query: ListAgentsQuery,
        reply: oneshot::Sender<Vec<AgentListing>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

#[derive(Clone, Debug)]
pub struct RegistryHandle {
    commands: mpsc::Sender<RegistryCommand>,
}

impl RegistryHandle {
    pub async fn register(
        &self,
        identity: AgentIdentity,
    ) -> Result<RegisterOutcome, RegistryServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(RegistryCommand::Register { identity, reply })
            .await?;
        response
            .await
            .map_err(|_| RegistryServiceError::ResponseDropped)?
            .map_err(RegistryServiceError::Registry)
    }

    pub async fn declare_capabilities(
        &self,
        agent_id: impl Into<AgentId>,
        capabilities: Vec<Capability>,
    ) -> Result<CapabilityUpdateOutcome, RegistryServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(RegistryCommand::DeclareCapabilities {
            agent_id: agent_id.into(),
            capabilities,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| RegistryServiceError::ResponseDropped)?
            .map_err(RegistryServiceError::Registry)
    }

    pub async fn deregister(
        &self,
        agent_id: impl Into<AgentId>,
    ) -> Result<bool, RegistryServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(RegistryCommand::Deregister {
            agent_id: agent_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| RegistryServiceError::ResponseDropped)
    }

    pub async fn mark_alive(
        &self,
        agent_id: impl Into<AgentId>,
    ) -> Result<(), RegistryServiceError> {
        self.send(RegistryCommand::MarkAlive {
            agent_id: agent_id.into(),
        })
        .await
    }

    pub async fn mark_timed_out(
        &self,
        agent_id: impl Into<AgentId>,
    ) -> Result<(), RegistryServiceError> {
        self.send(RegistryCommand::MarkTimedOut {
            agent_id: agent_id.into(),
        })
        .await
    }

    pub async fn apply_heartbeat_event(
        &self,
        event: HeartbeatEvent,
    ) -> Result<(), RegistryServiceError> {
        self.send(RegistryCommand::ApplyHeartbeatEvent { event })
            .await
    }

    pub async fn set_load(
        &self,
        agent_id: impl Into<AgentId>,
        current: u32,
    ) -> Result<(), RegistryServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(RegistryCommand::SetLoad {
            agent_id: agent_id.into(),
            current,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| RegistryServiceError::ResponseDropped)?
            .map_err(RegistryServiceError::Registry)
    }

    pub async fn discover(
        &self,
        query: DiscoveryQuery,
    ) -> Result<Vec<AgentCandidate>, RegistryServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(RegistryCommand::Discover { query, reply })
            .await?;
        response
            .await
            .map_err(|_| RegistryServiceError::ResponseDropped)
    }

    pub async fn list_agents(
        &self,
        query: ListAgentsQuery,
    ) -> Result<Vec<AgentListing>, RegistryServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(RegistryCommand::ListAgents { query, reply })
            .await?;
        response
            .await
            .map_err(|_| RegistryServiceError::ResponseDropped)
    }

    pub async fn shutdown(&self) -> Result<(), RegistryServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(RegistryCommand::Shutdown { reply }).await?;
        response
            .await
            .map_err(|_| RegistryServiceError::ResponseDropped)
    }

    async fn send(&self, command: RegistryCommand) -> Result<(), RegistryServiceError> {
        self.commands
            .send(command)
            .await
            .map_err(|_| RegistryServiceError::Stopped)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryServiceError {
    Registry(RegistryError),
    Stopped,
    ResponseDropped,
}

impl fmt::Display for RegistryServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryServiceError::Registry(error) => write!(f, "{error}"),
            RegistryServiceError::Stopped => f.write_str("registry service is stopped"),
            RegistryServiceError::ResponseDropped => {
                f.write_str("registry service dropped the response")
            }
        }
    }
}

impl Error for RegistryServiceError {}

pub struct RegistryService {
    core: RegistryCore,
    commands: mpsc::Receiver<RegistryCommand>,
}

impl RegistryService {
    pub fn spawn() -> RegistryHandle {
        Self::spawn_with_buffer(DEFAULT_COMMAND_BUFFER)
    }

    pub fn spawn_with_buffer(command_buffer: usize) -> RegistryHandle {
        assert!(
            command_buffer > 0,
            "registry command buffer must be greater than zero"
        );
        let (commands, receiver) = mpsc::channel(command_buffer);
        let service = Self {
            core: RegistryCore::new(),
            commands: receiver,
        };

        tokio::spawn(service.run());

        RegistryHandle { commands }
    }

    async fn run(mut self) {
        let mut shutdown_reply = None;

        while let Some(command) = self.commands.recv().await {
            if let Some(reply) = self.handle_command(command) {
                shutdown_reply = Some(reply);
                break;
            }
        }

        if let Some(reply) = shutdown_reply {
            let _ = reply.send(());
        }
    }

    fn handle_command(&mut self, command: RegistryCommand) -> Option<oneshot::Sender<()>> {
        match command {
            RegistryCommand::Register { identity, reply } => {
                let _ = reply.send(self.core.register(identity));
                None
            }
            RegistryCommand::DeclareCapabilities {
                agent_id,
                capabilities,
                reply,
            } => {
                let _ = reply.send(self.core.declare_capabilities(&agent_id, capabilities));
                None
            }
            RegistryCommand::Deregister { agent_id, reply } => {
                let _ = reply.send(self.core.deregister(&agent_id));
                None
            }
            RegistryCommand::MarkAlive { agent_id } => {
                if !self.core.mark_alive(&agent_id) {
                    eprintln!("ignored alive event for unknown or deregistered agent: {agent_id}");
                }
                None
            }
            RegistryCommand::MarkTimedOut { agent_id } => {
                if !self.core.mark_timed_out(&agent_id) {
                    eprintln!("ignored timeout event for non-alive agent: {agent_id}");
                }
                None
            }
            RegistryCommand::ApplyHeartbeatEvent { event } => {
                self.apply_heartbeat_event(event);
                None
            }
            RegistryCommand::SetLoad {
                agent_id,
                current,
                reply,
            } => {
                let _ = reply.send(self.core.set_load(&agent_id, current));
                None
            }
            RegistryCommand::Discover { query, reply } => {
                let _ = reply.send(self.core.discover(query));
                None
            }
            RegistryCommand::ListAgents { query, reply } => {
                let _ = reply.send(self.core.list_agents(query));
                None
            }
            RegistryCommand::Shutdown { reply } => Some(reply),
        }
    }

    fn apply_heartbeat_event(&mut self, event: HeartbeatEvent) {
        match event {
            HeartbeatEvent::AgentTimedOut { agent_id } => {
                if !self.core.mark_timed_out(&agent_id) {
                    eprintln!("ignored timeout event for non-alive agent: {agent_id}");
                }
            }
            HeartbeatEvent::AgentRecovered { agent_id } => {
                if !self.core.mark_alive(&agent_id) {
                    eprintln!(
                        "ignored recovered event for unknown or deregistered agent: {agent_id}"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::types::{
        AgentIdentity, Capability, CapabilityName, DiscoveryQuery, RegisterOutcome,
    };
    use super::*;

    fn agent(id: &str) -> AgentId {
        AgentId::from(id)
    }

    fn identity(id: &str) -> AgentIdentity {
        AgentIdentity {
            agent_id: agent(id),
            name: Some(id.to_string()),
            endpoint: Some(format!("https://{id}.example.test")),
            metadata: BTreeMap::new(),
        }
    }

    fn capability(name: &str, max_concurrency: u32) -> Capability {
        Capability::new(name, max_concurrency)
    }

    #[tokio::test]
    async fn service_registers_declares_and_discovers_agent() {
        let registry = RegistryService::spawn();

        let outcome = registry.register(identity("agent-1")).await.unwrap();
        registry
            .declare_capabilities(agent("agent-1"), vec![capability("rust", 2)])
            .await
            .unwrap();
        registry.mark_alive(agent("agent-1")).await.unwrap();

        let candidates = registry
            .discover(DiscoveryQuery::new("rust"))
            .await
            .unwrap();

        assert_eq!(outcome, RegisterOutcome::Registered);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].agent_id, agent("agent-1"));

        registry.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_lists_registered_agents() {
        let registry = RegistryService::spawn();

        registry.register(identity("agent-2")).await.unwrap();
        registry.register(identity("agent-1")).await.unwrap();
        registry.mark_alive(agent("agent-1")).await.unwrap();

        let listings = registry.list_agents(ListAgentsQuery::new()).await.unwrap();

        assert_eq!(listings.len(), 2);
        assert_eq!(listings[0].agent_id, agent("agent-1"));
        assert!(listings[0].alive);
        assert_eq!(listings[1].agent_id, agent("agent-2"));
        assert!(!listings[1].alive);

        registry.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn heartbeat_timeout_and_recovered_events_update_discovery() {
        let registry = RegistryService::spawn();

        registry.register(identity("agent-1")).await.unwrap();
        registry
            .declare_capabilities(agent("agent-1"), vec![capability("rust", 2)])
            .await
            .unwrap();
        registry
            .apply_heartbeat_event(HeartbeatEvent::AgentRecovered {
                agent_id: agent("agent-1"),
            })
            .await
            .unwrap();
        assert_eq!(
            registry
                .discover(DiscoveryQuery::new("rust"))
                .await
                .unwrap()
                .len(),
            1
        );

        registry
            .apply_heartbeat_event(HeartbeatEvent::AgentTimedOut {
                agent_id: agent("agent-1"),
            })
            .await
            .unwrap();
        assert!(
            registry
                .discover(DiscoveryQuery::new("rust"))
                .await
                .unwrap()
                .is_empty()
        );

        registry.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn unknown_heartbeat_event_does_not_stop_service() {
        let registry = RegistryService::spawn();

        registry
            .apply_heartbeat_event(HeartbeatEvent::AgentRecovered {
                agent_id: agent("missing"),
            })
            .await
            .unwrap();

        let outcome = registry.register(identity("agent-1")).await.unwrap();

        assert_eq!(outcome, RegisterOutcome::Registered);

        registry.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_returns_registry_errors() {
        let registry = RegistryService::spawn();

        registry.register(identity("agent-1")).await.unwrap();
        let error = registry
            .declare_capabilities(agent("agent-1"), vec![capability("rust", 0)])
            .await
            .unwrap_err();

        assert_eq!(
            error,
            RegistryServiceError::Registry(RegistryError::ZeroMaxConcurrency(
                CapabilityName::from("rust")
            ))
        );

        registry.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_stops_after_shutdown() {
        let registry = RegistryService::spawn();

        registry.shutdown().await.unwrap();

        assert_eq!(
            registry.register(identity("agent-1")).await.unwrap_err(),
            RegistryServiceError::Stopped
        );
    }
}
