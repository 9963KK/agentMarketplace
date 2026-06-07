use std::error::Error;
use std::fmt;

use crate::heartbeat::AgentId;

macro_rules! id_type {
    ($name:ident, $display:literal) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self::try_new(value).expect(concat!($display, " must not be empty"))
            }

            pub fn try_new(value: impl Into<String>) -> Result<Self, ChainError> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(ChainError::EmptyId($display));
                }

                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(ChainId, "chain id");
id_type!(TaskId, "task id");
id_type!(NodeId, "node id");
id_type!(ArtifactId, "artifact id");
id_type!(Hash, "hash");
id_type!(Signature, "signature");

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Timestamp(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactManifest {
    pub artifact_id: ArtifactId,
    pub root_hash: Hash,
    pub size_bytes: u64,
    pub content_type: String,
    pub created_by: AgentId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactRef {
    pub artifact_id: ArtifactId,
    pub root_hash: Hash,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HolderCommitment {
    pub artifact_id: ArtifactId,
    pub holder_agent: AgentId,
    pub retrieval_endpoint: String,
    pub expires_at: Timestamp,
    pub signature: Signature,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChainStatus {
    Open,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeStatus {
    Pending,
    Completed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskChain {
    pub chain_id: ChainId,
    pub task_id: TaskId,
    pub root_agent: AgentId,
    pub head: NodeId,
    pub status: ChainStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChainNode {
    pub node_id: NodeId,
    pub chain_id: ChainId,
    pub executor: AgentId,
    pub reviewers: Vec<AgentId>,
    pub previous: Option<NodeId>,
    pub next: Option<NodeId>,
    pub input: Option<ArtifactRef>,
    pub output: Option<ArtifactRef>,
    pub status: NodeStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChainSnapshot {
    pub chain: TaskChain,
    pub nodes: Vec<ChainNode>,
    pub artifacts: Vec<ArtifactManifest>,
    pub holders: Vec<HolderCommitment>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChainError {
    EmptyId(&'static str),
    EmptyContentType,
    EmptyHolderEndpoint,
    ZeroArtifactSize(ArtifactId),
    ChainNotFound(ChainId),
    NodeNotFound(NodeId),
    ArtifactNotFound(ArtifactId),
    ArtifactHashMismatch {
        artifact_id: ArtifactId,
        expected: Hash,
        actual: Hash,
    },
    ChainClosed(ChainId),
    NotChainHead {
        expected: NodeId,
        actual: NodeId,
    },
    DuplicateReviewer(AgentId),
    PreviousNodeMissingOutput(NodeId),
    InputDoesNotMatchPreviousOutput {
        previous: NodeId,
        expected: ArtifactRef,
        actual: ArtifactRef,
    },
    NodeAlreadyHasOutput(NodeId),
    NodeAlreadyCompleted(NodeId),
    FinalNodeMissingOutput(NodeId),
    DuplicateArtifact(ArtifactId),
    DuplicateHolder {
        artifact_id: ArtifactId,
        holder_agent: AgentId,
    },
}

impl fmt::Display for ChainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChainError::EmptyId(kind) => write!(f, "{kind} must not be empty"),
            ChainError::EmptyContentType => f.write_str("artifact content type must not be empty"),
            ChainError::EmptyHolderEndpoint => f.write_str("holder endpoint must not be empty"),
            ChainError::ZeroArtifactSize(artifact_id) => {
                write!(f, "artifact size must be greater than zero: {artifact_id}")
            }
            ChainError::ChainNotFound(chain_id) => write!(f, "chain not found: {chain_id}"),
            ChainError::NodeNotFound(node_id) => write!(f, "node not found: {node_id}"),
            ChainError::ArtifactNotFound(artifact_id) => {
                write!(f, "artifact not found: {artifact_id}")
            }
            ChainError::ArtifactHashMismatch {
                artifact_id,
                expected,
                actual,
            } => write!(
                f,
                "artifact hash mismatch for {artifact_id}: expected={expected}, actual={actual}"
            ),
            ChainError::ChainClosed(chain_id) => write!(f, "chain is closed: {chain_id}"),
            ChainError::NotChainHead { expected, actual } => {
                write!(
                    f,
                    "node is not chain head: expected={expected}, actual={actual}"
                )
            }
            ChainError::DuplicateReviewer(agent_id) => {
                write!(f, "duplicate reviewer: {agent_id}")
            }
            ChainError::PreviousNodeMissingOutput(node_id) => {
                write!(f, "previous node is missing output: {node_id}")
            }
            ChainError::InputDoesNotMatchPreviousOutput {
                previous,
                expected,
                actual,
            } => write!(
                f,
                "input does not match previous output for {previous}: expected={expected:?}, actual={actual:?}"
            ),
            ChainError::NodeAlreadyHasOutput(node_id) => {
                write!(f, "node already has output: {node_id}")
            }
            ChainError::NodeAlreadyCompleted(node_id) => {
                write!(f, "node already completed: {node_id}")
            }
            ChainError::FinalNodeMissingOutput(node_id) => {
                write!(f, "final node is missing output: {node_id}")
            }
            ChainError::DuplicateArtifact(artifact_id) => {
                write!(f, "duplicate artifact: {artifact_id}")
            }
            ChainError::DuplicateHolder {
                artifact_id,
                holder_agent,
            } => write!(
                f,
                "duplicate holder for artifact {artifact_id}: {holder_agent}"
            ),
        }
    }
}

impl Error for ChainError {}
