use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::heartbeat::AgentId;
use crate::types::{AssignmentId, OutputHash, Timestamp};

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct TokenHash(String);

impl TokenHash {
    pub fn new(value: impl Into<String>) -> Self {
        Self::try_new(value).expect("token hash must not be empty")
    }

    pub fn try_new(value: impl Into<String>) -> Result<Self, StorageError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(StorageError::EmptyTokenHash);
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for TokenHash {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for TokenHash {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for TokenHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    pub fn new(value: impl Into<String>) -> Self {
        Self::try_new(value).expect("idempotency key must not be empty")
    }

    pub fn try_new(value: impl Into<String>) -> Result<Self, StorageError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(StorageError::EmptyIdempotencyKey);
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for IdempotencyKey {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for IdempotencyKey {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuthCredential {
    pub agent_id: AgentId,
    pub token_hash: TokenHash,
    pub issued_at: Timestamp,
    pub revoked_at: Option<Timestamp>,
}

impl AuthCredential {
    pub fn new(
        agent_id: impl Into<AgentId>,
        token_hash: impl Into<TokenHash>,
        issued_at: Timestamp,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            token_hash: token_hash.into(),
            issued_at,
            revoked_at: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.revoked_at.is_none()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum IdempotentOperation {
    RegisterAgent,
    DeclareCapabilities,
    DeregisterAgent,
    CreateTask,
    AddParticipant,
    CreateSession,
    CreateAssignment,
    SubmitArtifact,
    RequestReview,
    SubmitReview,
    Deposit,
    Hold,
    ReleaseExecute,
    ReleaseReview,
    Refund,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum IdempotencyOutcome {
    Pending,
    Succeeded(String),
    Failed(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IdempotencyRecord {
    pub key: IdempotencyKey,
    pub caller_agent_id: AgentId,
    pub operation: IdempotentOperation,
    pub outcome: IdempotencyOutcome,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl IdempotencyRecord {
    pub fn new(
        key: impl Into<IdempotencyKey>,
        caller_agent_id: impl Into<AgentId>,
        operation: IdempotentOperation,
        created_at: Timestamp,
    ) -> Self {
        Self {
            key: key.into(),
            caller_agent_id: caller_agent_id.into(),
            operation,
            outcome: IdempotencyOutcome::Pending,
            created_at,
            updated_at: created_at,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum IdempotencyDecision {
    Started(IdempotencyRecord),
    Replay(IdempotencyRecord),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArtifactLocator {
    pub assignment_id: AssignmentId,
    pub manifest_hash: OutputHash,
    pub manifest_uri: String,
    pub producer_agent_id: AgentId,
    pub created_at: Timestamp,
}

impl ArtifactLocator {
    pub fn new(
        assignment_id: impl Into<AssignmentId>,
        manifest_hash: impl Into<OutputHash>,
        manifest_uri: impl Into<String>,
        producer_agent_id: impl Into<AgentId>,
        created_at: Timestamp,
    ) -> Result<Self, StorageError> {
        let manifest_uri = manifest_uri.into();
        if manifest_uri.trim().is_empty() {
            return Err(StorageError::EmptyManifestUri);
        }

        Ok(Self {
            assignment_id: assignment_id.into(),
            manifest_hash: manifest_hash.into(),
            manifest_uri,
            producer_agent_id: producer_agent_id.into(),
            created_at,
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum StoreOutcome {
    Stored,
    AlreadyExists,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StorageError {
    EmptyTokenHash,
    EmptyIdempotencyKey,
    EmptyManifestUri,
    CredentialNotFound(TokenHash),
    CredentialAlreadyExists(TokenHash),
    CredentialTimestampWentBackwards {
        token_hash: TokenHash,
        current: Timestamp,
        attempted: Timestamp,
    },
    IdempotencyConflict {
        key: IdempotencyKey,
        caller_agent_id: AgentId,
        existing_operation: IdempotentOperation,
        attempted_operation: IdempotentOperation,
    },
    IdempotencyNotFound {
        key: IdempotencyKey,
        caller_agent_id: AgentId,
    },
    IdempotencyTimestampWentBackwards {
        key: IdempotencyKey,
        caller_agent_id: AgentId,
        current: Timestamp,
        attempted: Timestamp,
    },
    IdempotencyOutcomeAlreadyRecorded {
        key: IdempotencyKey,
        caller_agent_id: AgentId,
        existing: IdempotencyOutcome,
        attempted: IdempotencyOutcome,
    },
    ArtifactLocatorConflict {
        assignment_id: AssignmentId,
    },
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::EmptyTokenHash => f.write_str("token hash must not be empty"),
            StorageError::EmptyIdempotencyKey => f.write_str("idempotency key must not be empty"),
            StorageError::EmptyManifestUri => f.write_str("manifest uri must not be empty"),
            StorageError::CredentialNotFound(token_hash) => {
                write!(f, "credential not found: {token_hash}")
            }
            StorageError::CredentialAlreadyExists(token_hash) => {
                write!(f, "credential already exists: {token_hash}")
            }
            StorageError::CredentialTimestampWentBackwards {
                token_hash,
                current,
                attempted,
            } => write!(
                f,
                "credential timestamp went backwards: {token_hash}, current={}, attempted={}",
                current.0, attempted.0
            ),
            StorageError::IdempotencyConflict {
                key,
                caller_agent_id,
                existing_operation,
                attempted_operation,
            } => write!(
                f,
                "idempotency conflict: key={key}, caller={caller_agent_id}, existing={existing_operation:?}, attempted={attempted_operation:?}"
            ),
            StorageError::IdempotencyNotFound {
                key,
                caller_agent_id,
            } => write!(
                f,
                "idempotency record not found: key={key}, caller={caller_agent_id}"
            ),
            StorageError::IdempotencyTimestampWentBackwards {
                key,
                caller_agent_id,
                current,
                attempted,
            } => write!(
                f,
                "idempotency timestamp went backwards: key={key}, caller={caller_agent_id}, current={}, attempted={}",
                current.0, attempted.0
            ),
            StorageError::IdempotencyOutcomeAlreadyRecorded {
                key,
                caller_agent_id,
                existing,
                attempted,
            } => write!(
                f,
                "idempotency outcome already recorded: key={key}, caller={caller_agent_id}, existing={existing:?}, attempted={attempted:?}"
            ),
            StorageError::ArtifactLocatorConflict { assignment_id } => {
                write!(
                    f,
                    "artifact locator already exists with different data: {assignment_id}"
                )
            }
        }
    }
}

impl Error for StorageError {}
