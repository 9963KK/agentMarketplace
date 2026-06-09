use std::collections::HashMap;

use crate::heartbeat::AgentId;
use crate::types::{AssignmentId, Timestamp};

use super::types::{
    ArtifactLocator, AuthCredential, IdempotencyDecision, IdempotencyKey, IdempotencyOutcome,
    IdempotencyRecord, IdempotentOperation, StorageError, StoreOutcome, TokenHash,
};

#[derive(Debug, Default)]
pub struct StorageCore {
    credentials_by_token: HashMap<TokenHash, AuthCredential>,
    idempotency: HashMap<(AgentId, IdempotencyKey), IdempotencyRecord>,
    artifact_locators: HashMap<AssignmentId, ArtifactLocator>,
}

impl StorageCore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn store_credential(
        &mut self,
        credential: AuthCredential,
    ) -> Result<StoreOutcome, StorageError> {
        if let Some(existing) = self.credentials_by_token.get(&credential.token_hash) {
            if existing == &credential {
                return Ok(StoreOutcome::AlreadyExists);
            }
            return Err(StorageError::CredentialAlreadyExists(credential.token_hash));
        }

        self.credentials_by_token
            .insert(credential.token_hash.clone(), credential);
        Ok(StoreOutcome::Stored)
    }

    pub fn authenticate(&self, token_hash: &TokenHash) -> Option<AgentId> {
        self.credentials_by_token
            .get(token_hash)
            .filter(|credential| credential.is_active())
            .map(|credential| credential.agent_id.clone())
    }

    pub fn credential(&self, token_hash: &TokenHash) -> Option<&AuthCredential> {
        self.credentials_by_token.get(token_hash)
    }

    pub fn revoke_credential(
        &mut self,
        token_hash: &TokenHash,
        revoked_at: Timestamp,
    ) -> Result<bool, StorageError> {
        let credential = self
            .credentials_by_token
            .get_mut(token_hash)
            .ok_or_else(|| StorageError::CredentialNotFound(token_hash.clone()))?;
        if let Some(current) = credential.revoked_at {
            if revoked_at < current {
                return Err(StorageError::CredentialTimestampWentBackwards {
                    token_hash: token_hash.clone(),
                    current,
                    attempted: revoked_at,
                });
            }
            return Ok(false);
        }
        if revoked_at < credential.issued_at {
            return Err(StorageError::CredentialTimestampWentBackwards {
                token_hash: token_hash.clone(),
                current: credential.issued_at,
                attempted: revoked_at,
            });
        }

        credential.revoked_at = Some(revoked_at);
        Ok(true)
    }

    pub fn begin_idempotency(
        &mut self,
        caller_agent_id: AgentId,
        key: IdempotencyKey,
        operation: IdempotentOperation,
        at: Timestamp,
    ) -> Result<IdempotencyDecision, StorageError> {
        let index = (caller_agent_id.clone(), key.clone());
        if let Some(existing) = self.idempotency.get(&index) {
            if existing.operation != operation {
                return Err(StorageError::IdempotencyConflict {
                    key,
                    caller_agent_id,
                    existing_operation: existing.operation,
                    attempted_operation: operation,
                });
            }
            return Ok(IdempotencyDecision::Replay(existing.clone()));
        }

        let record = IdempotencyRecord::new(key, caller_agent_id, operation, at);
        self.idempotency.insert(index, record.clone());
        Ok(IdempotencyDecision::Started(record))
    }

    pub fn finish_idempotency(
        &mut self,
        caller_agent_id: AgentId,
        key: IdempotencyKey,
        operation: IdempotentOperation,
        outcome: IdempotencyOutcome,
        at: Timestamp,
    ) -> Result<IdempotencyRecord, StorageError> {
        let index = (caller_agent_id.clone(), key.clone());
        let record =
            self.idempotency
                .get_mut(&index)
                .ok_or_else(|| StorageError::IdempotencyNotFound {
                    key: key.clone(),
                    caller_agent_id: caller_agent_id.clone(),
                })?;
        if record.operation != operation {
            return Err(StorageError::IdempotencyConflict {
                key,
                caller_agent_id,
                existing_operation: record.operation,
                attempted_operation: operation,
            });
        }
        if at < record.updated_at {
            return Err(StorageError::IdempotencyTimestampWentBackwards {
                key,
                caller_agent_id,
                current: record.updated_at,
                attempted: at,
            });
        }
        if record.outcome != IdempotencyOutcome::Pending {
            if record.outcome == outcome {
                return Ok(record.clone());
            }
            return Err(StorageError::IdempotencyOutcomeAlreadyRecorded {
                key,
                caller_agent_id,
                existing: record.outcome.clone(),
                attempted: outcome,
            });
        }

        record.outcome = outcome;
        record.updated_at = at;
        Ok(record.clone())
    }

    pub fn idempotency_record(
        &self,
        caller_agent_id: &AgentId,
        key: &IdempotencyKey,
    ) -> Option<&IdempotencyRecord> {
        self.idempotency
            .get(&(caller_agent_id.clone(), key.clone()))
    }

    pub fn store_artifact_locator(
        &mut self,
        locator: ArtifactLocator,
    ) -> Result<StoreOutcome, StorageError> {
        if let Some(existing) = self.artifact_locators.get(&locator.assignment_id) {
            if existing == &locator {
                return Ok(StoreOutcome::AlreadyExists);
            }
            return Err(StorageError::ArtifactLocatorConflict {
                assignment_id: locator.assignment_id,
            });
        }

        self.artifact_locators
            .insert(locator.assignment_id.clone(), locator);
        Ok(StoreOutcome::Stored)
    }

    pub fn artifact_locator(&self, assignment_id: &AssignmentId) -> Option<&ArtifactLocator> {
        self.artifact_locators.get(assignment_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::OutputHash;

    fn agent(id: &str) -> AgentId {
        AgentId::from(id)
    }

    fn token(value: &str) -> TokenHash {
        TokenHash::from(value)
    }

    fn key(value: &str) -> IdempotencyKey {
        IdempotencyKey::from(value)
    }

    #[test]
    fn authenticates_active_credential_and_rejects_revoked_token() {
        let mut core = StorageCore::new();
        let token_hash = token("token-hash-1");
        let credential = AuthCredential::new(agent("agent-1"), token_hash.clone(), Timestamp(10));

        assert_eq!(
            core.store_credential(credential.clone()).unwrap(),
            StoreOutcome::Stored
        );
        assert_eq!(
            core.store_credential(credential).unwrap(),
            StoreOutcome::AlreadyExists
        );
        assert_eq!(core.authenticate(&token_hash), Some(agent("agent-1")));

        assert!(core.revoke_credential(&token_hash, Timestamp(11)).unwrap());
        assert_eq!(core.authenticate(&token_hash), None);
        assert!(!core.revoke_credential(&token_hash, Timestamp(12)).unwrap());
    }

    #[test]
    fn rejects_duplicate_token_with_different_credential() {
        let mut core = StorageCore::new();
        let token_hash = token("token-hash-1");
        core.store_credential(AuthCredential::new(
            agent("agent-1"),
            token_hash.clone(),
            Timestamp(10),
        ))
        .unwrap();

        let error = core
            .store_credential(AuthCredential::new(
                agent("agent-2"),
                token_hash.clone(),
                Timestamp(10),
            ))
            .unwrap_err();

        assert_eq!(error, StorageError::CredentialAlreadyExists(token_hash));
    }

    #[test]
    fn idempotency_replays_same_operation_and_rejects_conflicts() {
        let mut core = StorageCore::new();
        let caller = agent("agent-1");
        let key = key("request-1");

        let first = core
            .begin_idempotency(
                caller.clone(),
                key.clone(),
                IdempotentOperation::CreateTask,
                Timestamp(1),
            )
            .unwrap();
        assert!(matches!(first, IdempotencyDecision::Started(_)));

        let replay = core
            .begin_idempotency(
                caller.clone(),
                key.clone(),
                IdempotentOperation::CreateTask,
                Timestamp(2),
            )
            .unwrap();
        assert!(matches!(replay, IdempotencyDecision::Replay(_)));

        let error = core
            .begin_idempotency(
                caller.clone(),
                key.clone(),
                IdempotentOperation::Hold,
                Timestamp(3),
            )
            .unwrap_err();
        assert_eq!(
            error,
            StorageError::IdempotencyConflict {
                key,
                caller_agent_id: caller,
                existing_operation: IdempotentOperation::CreateTask,
                attempted_operation: IdempotentOperation::Hold,
            }
        );
    }

    #[test]
    fn idempotency_finish_records_stable_outcome() {
        let mut core = StorageCore::new();
        let caller = agent("agent-1");
        let key = key("request-1");
        core.begin_idempotency(
            caller.clone(),
            key.clone(),
            IdempotentOperation::SubmitArtifact,
            Timestamp(1),
        )
        .unwrap();

        let record = core
            .finish_idempotency(
                caller.clone(),
                key.clone(),
                IdempotentOperation::SubmitArtifact,
                IdempotencyOutcome::Succeeded("assignment-1".to_string()),
                Timestamp(2),
            )
            .unwrap();

        assert_eq!(
            record.outcome,
            IdempotencyOutcome::Succeeded("assignment-1".to_string())
        );
        assert_eq!(
            core.idempotency_record(&caller, &key).unwrap().updated_at,
            Timestamp(2)
        );

        let replay = core
            .finish_idempotency(
                caller.clone(),
                key.clone(),
                IdempotentOperation::SubmitArtifact,
                IdempotencyOutcome::Succeeded("assignment-1".to_string()),
                Timestamp(3),
            )
            .unwrap();
        assert_eq!(
            replay.outcome,
            IdempotencyOutcome::Succeeded("assignment-1".to_string())
        );

        let error = core
            .finish_idempotency(
                caller.clone(),
                key.clone(),
                IdempotentOperation::SubmitArtifact,
                IdempotencyOutcome::Failed("different failure".to_string()),
                Timestamp(4),
            )
            .unwrap_err();
        assert_eq!(
            error,
            StorageError::IdempotencyOutcomeAlreadyRecorded {
                key,
                caller_agent_id: caller,
                existing: IdempotencyOutcome::Succeeded("assignment-1".to_string()),
                attempted: IdempotencyOutcome::Failed("different failure".to_string()),
            }
        );
    }

    #[test]
    fn artifact_locator_is_immutable_per_assignment() {
        let mut core = StorageCore::new();
        let locator = ArtifactLocator::new(
            "assignment-1",
            OutputHash::from("sha256:aaa"),
            "https://agent.example/manifests/a.json",
            "agent-1",
            Timestamp(1),
        )
        .unwrap();

        assert_eq!(
            core.store_artifact_locator(locator.clone()).unwrap(),
            StoreOutcome::Stored
        );
        assert_eq!(
            core.store_artifact_locator(locator.clone()).unwrap(),
            StoreOutcome::AlreadyExists
        );
        assert_eq!(
            core.artifact_locator(&AssignmentId::from("assignment-1")),
            Some(&locator)
        );

        let changed = ArtifactLocator::new(
            "assignment-1",
            OutputHash::from("sha256:bbb"),
            "https://agent.example/manifests/b.json",
            "agent-1",
            Timestamp(1),
        )
        .unwrap();
        assert_eq!(
            core.store_artifact_locator(changed).unwrap_err(),
            StorageError::ArtifactLocatorConflict {
                assignment_id: AssignmentId::from("assignment-1"),
            }
        );
    }
}
