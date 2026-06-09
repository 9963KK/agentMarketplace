use std::error::Error;
use std::fmt;

use tokio::sync::{mpsc, oneshot};

use crate::heartbeat::AgentId;
use crate::types::{AssignmentId, Timestamp};

use super::StorageCore;
use super::types::{
    ArtifactLocator, AuthCredential, IdempotencyDecision, IdempotencyKey, IdempotencyOutcome,
    IdempotencyRecord, IdempotentOperation, StorageError, StoreOutcome, TokenHash,
};

const DEFAULT_COMMAND_BUFFER: usize = 128;

#[derive(Debug)]
pub enum StorageCommand {
    StoreCredential {
        credential: AuthCredential,
        reply: oneshot::Sender<Result<StoreOutcome, StorageError>>,
    },
    Authenticate {
        token_hash: TokenHash,
        reply: oneshot::Sender<Option<AgentId>>,
    },
    RevokeCredential {
        token_hash: TokenHash,
        revoked_at: Timestamp,
        reply: oneshot::Sender<Result<bool, StorageError>>,
    },
    BeginIdempotency {
        caller_agent_id: AgentId,
        key: IdempotencyKey,
        operation: IdempotentOperation,
        at: Timestamp,
        reply: oneshot::Sender<Result<IdempotencyDecision, StorageError>>,
    },
    FinishIdempotency {
        caller_agent_id: AgentId,
        key: IdempotencyKey,
        operation: IdempotentOperation,
        outcome: IdempotencyOutcome,
        at: Timestamp,
        reply: oneshot::Sender<Result<IdempotencyRecord, StorageError>>,
    },
    StoreArtifactLocator {
        locator: ArtifactLocator,
        reply: oneshot::Sender<Result<StoreOutcome, StorageError>>,
    },
    GetArtifactLocator {
        assignment_id: AssignmentId,
        reply: oneshot::Sender<Option<ArtifactLocator>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

#[derive(Clone, Debug)]
pub struct StorageHandle {
    commands: mpsc::Sender<StorageCommand>,
}

impl StorageHandle {
    pub async fn store_credential(
        &self,
        credential: AuthCredential,
    ) -> Result<StoreOutcome, StorageServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(StorageCommand::StoreCredential { credential, reply })
            .await?;
        response
            .await
            .map_err(|_| StorageServiceError::ResponseDropped)?
            .map_err(StorageServiceError::Storage)
    }

    pub async fn authenticate(
        &self,
        token_hash: impl Into<TokenHash>,
    ) -> Result<Option<AgentId>, StorageServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(StorageCommand::Authenticate {
            token_hash: token_hash.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| StorageServiceError::ResponseDropped)
    }

    pub async fn revoke_credential(
        &self,
        token_hash: impl Into<TokenHash>,
        revoked_at: Timestamp,
    ) -> Result<bool, StorageServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(StorageCommand::RevokeCredential {
            token_hash: token_hash.into(),
            revoked_at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| StorageServiceError::ResponseDropped)?
            .map_err(StorageServiceError::Storage)
    }

    pub async fn begin_idempotency(
        &self,
        caller_agent_id: impl Into<AgentId>,
        key: impl Into<IdempotencyKey>,
        operation: IdempotentOperation,
        at: Timestamp,
    ) -> Result<IdempotencyDecision, StorageServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(StorageCommand::BeginIdempotency {
            caller_agent_id: caller_agent_id.into(),
            key: key.into(),
            operation,
            at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| StorageServiceError::ResponseDropped)?
            .map_err(StorageServiceError::Storage)
    }

    pub async fn finish_idempotency(
        &self,
        caller_agent_id: impl Into<AgentId>,
        key: impl Into<IdempotencyKey>,
        operation: IdempotentOperation,
        outcome: IdempotencyOutcome,
        at: Timestamp,
    ) -> Result<IdempotencyRecord, StorageServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(StorageCommand::FinishIdempotency {
            caller_agent_id: caller_agent_id.into(),
            key: key.into(),
            operation,
            outcome,
            at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| StorageServiceError::ResponseDropped)?
            .map_err(StorageServiceError::Storage)
    }

    pub async fn store_artifact_locator(
        &self,
        locator: ArtifactLocator,
    ) -> Result<StoreOutcome, StorageServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(StorageCommand::StoreArtifactLocator { locator, reply })
            .await?;
        response
            .await
            .map_err(|_| StorageServiceError::ResponseDropped)?
            .map_err(StorageServiceError::Storage)
    }

    pub async fn get_artifact_locator(
        &self,
        assignment_id: impl Into<AssignmentId>,
    ) -> Result<Option<ArtifactLocator>, StorageServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(StorageCommand::GetArtifactLocator {
            assignment_id: assignment_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| StorageServiceError::ResponseDropped)
    }

    pub async fn shutdown(&self) -> Result<(), StorageServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(StorageCommand::Shutdown { reply }).await?;
        response
            .await
            .map_err(|_| StorageServiceError::ResponseDropped)
    }

    async fn send(&self, command: StorageCommand) -> Result<(), StorageServiceError> {
        self.commands
            .send(command)
            .await
            .map_err(|_| StorageServiceError::Stopped)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StorageServiceError {
    Storage(StorageError),
    Stopped,
    ResponseDropped,
}

impl fmt::Display for StorageServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageServiceError::Storage(error) => write!(f, "{error}"),
            StorageServiceError::Stopped => f.write_str("storage service is stopped"),
            StorageServiceError::ResponseDropped => {
                f.write_str("storage service dropped the response")
            }
        }
    }
}

impl Error for StorageServiceError {}

pub struct StorageService {
    core: StorageCore,
    commands: mpsc::Receiver<StorageCommand>,
}

impl StorageService {
    pub fn spawn() -> StorageHandle {
        Self::spawn_with_buffer(DEFAULT_COMMAND_BUFFER)
    }

    pub fn spawn_with_buffer(command_buffer: usize) -> StorageHandle {
        assert!(
            command_buffer > 0,
            "storage command buffer must be greater than zero"
        );
        let (commands, receiver) = mpsc::channel(command_buffer);
        let service = Self {
            core: StorageCore::new(),
            commands: receiver,
        };

        tokio::spawn(service.run());

        StorageHandle { commands }
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

    fn handle_command(&mut self, command: StorageCommand) -> Option<oneshot::Sender<()>> {
        match command {
            StorageCommand::StoreCredential { credential, reply } => {
                let _ = reply.send(self.core.store_credential(credential));
                None
            }
            StorageCommand::Authenticate { token_hash, reply } => {
                let _ = reply.send(self.core.authenticate(&token_hash));
                None
            }
            StorageCommand::RevokeCredential {
                token_hash,
                revoked_at,
                reply,
            } => {
                let _ = reply.send(self.core.revoke_credential(&token_hash, revoked_at));
                None
            }
            StorageCommand::BeginIdempotency {
                caller_agent_id,
                key,
                operation,
                at,
                reply,
            } => {
                let _ =
                    reply.send(
                        self.core
                            .begin_idempotency(caller_agent_id, key, operation, at),
                    );
                None
            }
            StorageCommand::FinishIdempotency {
                caller_agent_id,
                key,
                operation,
                outcome,
                at,
                reply,
            } => {
                let _ = reply.send(self.core.finish_idempotency(
                    caller_agent_id,
                    key,
                    operation,
                    outcome,
                    at,
                ));
                None
            }
            StorageCommand::StoreArtifactLocator { locator, reply } => {
                let _ = reply.send(self.core.store_artifact_locator(locator));
                None
            }
            StorageCommand::GetArtifactLocator {
                assignment_id,
                reply,
            } => {
                let _ = reply.send(self.core.artifact_locator(&assignment_id).cloned());
                None
            }
            StorageCommand::Shutdown { reply } => Some(reply),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::OutputHash;

    #[tokio::test]
    async fn service_round_trips_auth_idempotency_and_artifact_locator() {
        let storage = StorageService::spawn();
        storage
            .store_credential(AuthCredential::new(
                AgentId::from("agent-1"),
                TokenHash::from("token-hash-1"),
                Timestamp(1),
            ))
            .await
            .unwrap();

        assert_eq!(
            storage.authenticate("token-hash-1").await.unwrap(),
            Some(AgentId::from("agent-1"))
        );
        assert!(matches!(
            storage
                .begin_idempotency(
                    "agent-1",
                    "request-1",
                    IdempotentOperation::SubmitArtifact,
                    Timestamp(2),
                )
                .await
                .unwrap(),
            IdempotencyDecision::Started(_)
        ));
        storage
            .finish_idempotency(
                "agent-1",
                "request-1",
                IdempotentOperation::SubmitArtifact,
                IdempotencyOutcome::Succeeded("assignment-1".to_string()),
                Timestamp(3),
            )
            .await
            .unwrap();

        let locator = ArtifactLocator::new(
            "assignment-1",
            OutputHash::from("sha256:aaa"),
            "https://agent.example/manifests/a.json",
            "agent-1",
            Timestamp(4),
        )
        .unwrap();
        storage
            .store_artifact_locator(locator.clone())
            .await
            .unwrap();
        assert_eq!(
            storage.get_artifact_locator("assignment-1").await.unwrap(),
            Some(locator)
        );

        storage.shutdown().await.unwrap();
    }
}
