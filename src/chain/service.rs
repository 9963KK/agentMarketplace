use std::error::Error;
use std::fmt;

use tokio::sync::{mpsc, oneshot};

use crate::heartbeat::AgentId;

use super::ChainCore;
use super::types::{
    ArtifactManifest, ArtifactRef, ChainError, ChainId, ChainSnapshot, HolderCommitment, NodeId,
    TaskId,
};

const DEFAULT_COMMAND_BUFFER: usize = 128;

#[derive(Debug)]
pub enum ChainCommand {
    CreateChain {
        task_id: TaskId,
        root_agent: AgentId,
        reviewers: Vec<AgentId>,
        reply: oneshot::Sender<Result<ChainId, ChainError>>,
    },
    RegisterArtifact {
        manifest: ArtifactManifest,
        reply: oneshot::Sender<Result<(), ChainError>>,
    },
    AddHolder {
        commitment: HolderCommitment,
        reply: oneshot::Sender<Result<(), ChainError>>,
    },
    SubmitOutput {
        node_id: NodeId,
        output: ArtifactRef,
        reply: oneshot::Sender<Result<(), ChainError>>,
    },
    AppendNode {
        chain_id: ChainId,
        previous: NodeId,
        executor: AgentId,
        reviewers: Vec<AgentId>,
        input: ArtifactRef,
        reply: oneshot::Sender<Result<NodeId, ChainError>>,
    },
    AssignExecutor {
        node_id: NodeId,
        executor: AgentId,
        reply: oneshot::Sender<Result<(), ChainError>>,
    },
    AssignReviewers {
        node_id: NodeId,
        reviewers: Vec<AgentId>,
        reply: oneshot::Sender<Result<(), ChainError>>,
    },
    CloseChain {
        chain_id: ChainId,
        reply: oneshot::Sender<Result<(), ChainError>>,
    },
    GetChain {
        chain_id: ChainId,
        reply: oneshot::Sender<Option<ChainSnapshot>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

#[derive(Clone, Debug)]
pub struct ChainHandle {
    commands: mpsc::Sender<ChainCommand>,
}

impl ChainHandle {
    pub async fn create_chain(
        &self,
        task_id: impl Into<TaskId>,
        root_agent: impl Into<AgentId>,
        reviewers: Vec<AgentId>,
    ) -> Result<ChainId, ChainServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ChainCommand::CreateChain {
            task_id: task_id.into(),
            root_agent: root_agent.into(),
            reviewers,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| ChainServiceError::ResponseDropped)?
            .map_err(ChainServiceError::Chain)
    }

    pub async fn register_artifact(
        &self,
        manifest: ArtifactManifest,
    ) -> Result<(), ChainServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ChainCommand::RegisterArtifact { manifest, reply })
            .await?;
        response
            .await
            .map_err(|_| ChainServiceError::ResponseDropped)?
            .map_err(ChainServiceError::Chain)
    }

    pub async fn add_holder(&self, commitment: HolderCommitment) -> Result<(), ChainServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ChainCommand::AddHolder { commitment, reply })
            .await?;
        response
            .await
            .map_err(|_| ChainServiceError::ResponseDropped)?
            .map_err(ChainServiceError::Chain)
    }

    pub async fn submit_output(
        &self,
        node_id: impl Into<NodeId>,
        output: ArtifactRef,
    ) -> Result<(), ChainServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ChainCommand::SubmitOutput {
            node_id: node_id.into(),
            output,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| ChainServiceError::ResponseDropped)?
            .map_err(ChainServiceError::Chain)
    }

    pub async fn append_node(
        &self,
        chain_id: impl Into<ChainId>,
        previous: impl Into<NodeId>,
        executor: impl Into<AgentId>,
        reviewers: Vec<AgentId>,
        input: ArtifactRef,
    ) -> Result<NodeId, ChainServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ChainCommand::AppendNode {
            chain_id: chain_id.into(),
            previous: previous.into(),
            executor: executor.into(),
            reviewers,
            input,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| ChainServiceError::ResponseDropped)?
            .map_err(ChainServiceError::Chain)
    }

    pub async fn assign_executor(
        &self,
        node_id: impl Into<NodeId>,
        executor: impl Into<AgentId>,
    ) -> Result<(), ChainServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ChainCommand::AssignExecutor {
            node_id: node_id.into(),
            executor: executor.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| ChainServiceError::ResponseDropped)?
            .map_err(ChainServiceError::Chain)
    }

    pub async fn assign_reviewers(
        &self,
        node_id: impl Into<NodeId>,
        reviewers: Vec<AgentId>,
    ) -> Result<(), ChainServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ChainCommand::AssignReviewers {
            node_id: node_id.into(),
            reviewers,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| ChainServiceError::ResponseDropped)?
            .map_err(ChainServiceError::Chain)
    }

    pub async fn close_chain(&self, chain_id: impl Into<ChainId>) -> Result<(), ChainServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ChainCommand::CloseChain {
            chain_id: chain_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| ChainServiceError::ResponseDropped)?
            .map_err(ChainServiceError::Chain)
    }

    pub async fn get_chain(
        &self,
        chain_id: impl Into<ChainId>,
    ) -> Result<Option<ChainSnapshot>, ChainServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ChainCommand::GetChain {
            chain_id: chain_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| ChainServiceError::ResponseDropped)
    }

    pub async fn shutdown(&self) -> Result<(), ChainServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ChainCommand::Shutdown { reply }).await?;
        response
            .await
            .map_err(|_| ChainServiceError::ResponseDropped)
    }

    async fn send(&self, command: ChainCommand) -> Result<(), ChainServiceError> {
        self.commands
            .send(command)
            .await
            .map_err(|_| ChainServiceError::Stopped)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChainServiceError {
    Chain(ChainError),
    Stopped,
    ResponseDropped,
}

impl fmt::Display for ChainServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChainServiceError::Chain(error) => write!(f, "{error}"),
            ChainServiceError::Stopped => f.write_str("chain service is stopped"),
            ChainServiceError::ResponseDropped => f.write_str("chain service dropped the response"),
        }
    }
}

impl Error for ChainServiceError {}

pub struct ChainService {
    core: ChainCore,
    commands: mpsc::Receiver<ChainCommand>,
}

impl ChainService {
    pub fn spawn() -> ChainHandle {
        Self::spawn_with_buffer(DEFAULT_COMMAND_BUFFER)
    }

    pub fn spawn_with_buffer(command_buffer: usize) -> ChainHandle {
        assert!(
            command_buffer > 0,
            "chain command buffer must be greater than zero"
        );
        let (commands, receiver) = mpsc::channel(command_buffer);
        let service = Self {
            core: ChainCore::new(),
            commands: receiver,
        };

        tokio::spawn(service.run());

        ChainHandle { commands }
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

    fn handle_command(&mut self, command: ChainCommand) -> Option<oneshot::Sender<()>> {
        match command {
            ChainCommand::CreateChain {
                task_id,
                root_agent,
                reviewers,
                reply,
            } => {
                let _ = reply.send(self.core.create_chain(task_id, root_agent, reviewers));
                None
            }
            ChainCommand::RegisterArtifact { manifest, reply } => {
                let _ = reply.send(self.core.register_artifact(manifest));
                None
            }
            ChainCommand::AddHolder { commitment, reply } => {
                let _ = reply.send(self.core.add_holder(commitment));
                None
            }
            ChainCommand::SubmitOutput {
                node_id,
                output,
                reply,
            } => {
                let _ = reply.send(self.core.submit_output(&node_id, output));
                None
            }
            ChainCommand::AppendNode {
                chain_id,
                previous,
                executor,
                reviewers,
                input,
                reply,
            } => {
                let _ = reply.send(
                    self.core
                        .append_node(&chain_id, previous, executor, reviewers, input),
                );
                None
            }
            ChainCommand::AssignExecutor {
                node_id,
                executor,
                reply,
            } => {
                let _ = reply.send(self.core.assign_executor(&node_id, executor));
                None
            }
            ChainCommand::AssignReviewers {
                node_id,
                reviewers,
                reply,
            } => {
                let _ = reply.send(self.core.assign_reviewers(&node_id, reviewers));
                None
            }
            ChainCommand::CloseChain { chain_id, reply } => {
                let _ = reply.send(self.core.close_chain(&chain_id));
                None
            }
            ChainCommand::GetChain { chain_id, reply } => {
                let _ = reply.send(self.core.get_chain(&chain_id));
                None
            }
            ChainCommand::Shutdown { reply } => Some(reply),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::{ArtifactId, Hash, Signature, Timestamp};

    fn agent(id: &str) -> AgentId {
        AgentId::from(id)
    }

    fn artifact_id(id: &str) -> ArtifactId {
        ArtifactId::from(id)
    }

    fn hash(value: &str) -> Hash {
        Hash::from(value)
    }

    fn artifact_ref(id: &str, root_hash: &str) -> ArtifactRef {
        ArtifactRef {
            artifact_id: artifact_id(id),
            root_hash: hash(root_hash),
        }
    }

    fn manifest(id: &str, root_hash: &str, creator: &str) -> ArtifactManifest {
        ArtifactManifest {
            artifact_id: artifact_id(id),
            root_hash: hash(root_hash),
            size_bytes: 128,
            content_type: "application/json".to_string(),
            created_by: agent(creator),
        }
    }

    fn holder(id: &str, holder: &str) -> HolderCommitment {
        HolderCommitment {
            artifact_id: artifact_id(id),
            holder_agent: agent(holder),
            retrieval_endpoint: format!("https://{holder}.example.test/artifacts/{id}"),
            expires_at: Timestamp(10),
            signature: Signature::from(format!("sig-{holder}-{id}")),
        }
    }

    #[tokio::test]
    async fn service_records_chain_artifacts_and_holders() {
        let chain = ChainService::spawn();

        let chain_id = chain
            .create_chain("task-1", "agent-a", vec![agent("reviewer-a")])
            .await
            .unwrap();
        let root = chain
            .get_chain(chain_id.clone())
            .await
            .unwrap()
            .unwrap()
            .chain
            .head;

        chain
            .register_artifact(manifest("artifact-a", "hash-a", "agent-a"))
            .await
            .unwrap();
        chain
            .add_holder(holder("artifact-a", "agent-a"))
            .await
            .unwrap();
        chain
            .submit_output(root.clone(), artifact_ref("artifact-a", "hash-a"))
            .await
            .unwrap();
        let node_b = chain
            .append_node(
                chain_id.clone(),
                root,
                "agent-b",
                vec![agent("reviewer-b")],
                artifact_ref("artifact-a", "hash-a"),
            )
            .await
            .unwrap();
        chain
            .assign_reviewers(node_b.clone(), vec![agent("reviewer-c")])
            .await
            .unwrap();

        let snapshot = chain.get_chain(chain_id).await.unwrap().unwrap();
        let head = snapshot.nodes.last().unwrap();

        assert_eq!(snapshot.nodes.len(), 2);
        assert_eq!(snapshot.chain.head, node_b);
        assert_eq!(head.reviewers, vec![agent("reviewer-c")]);
        assert_eq!(snapshot.artifacts.len(), 1);
        assert_eq!(snapshot.holders.len(), 1);

        chain.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_returns_chain_errors() {
        let chain = ChainService::spawn();
        let chain_id = chain
            .create_chain("task-1", "agent-a", vec![agent("reviewer-a")])
            .await
            .unwrap();
        let root = chain
            .get_chain(chain_id.clone())
            .await
            .unwrap()
            .unwrap()
            .chain
            .head;

        let error = chain
            .append_node(
                chain_id,
                root.clone(),
                "agent-b",
                vec![agent("reviewer-b")],
                artifact_ref("artifact-a", "hash-a"),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error,
            ChainServiceError::Chain(ChainError::ArtifactNotFound(artifact_id("artifact-a")))
        );

        chain.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_stops_after_shutdown() {
        let chain = ChainService::spawn();

        chain.shutdown().await.unwrap();

        assert_eq!(
            chain
                .create_chain("task-1", "agent-a", Vec::new())
                .await
                .unwrap_err(),
            ChainServiceError::Stopped
        );
    }
}
