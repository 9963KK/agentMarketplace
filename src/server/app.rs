use std::env;
use std::fs::File;
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::artifact::{ArtifactManifest, sha256_digest};
use crate::heartbeat::{AgentId, HeartbeatConfig, HeartbeatHandle, HeartbeatService};
use crate::livesession::{Assignment, AssignmentKind, LiveSessionHandle, LiveSessionService};
use crate::registry::{
    AgentCandidate, AgentIdentity, AgentListing, Capability, DiscoveryQuery, ListAgentsQuery,
    RegistryHandle, RegistryService,
};
use crate::relay::{
    RelayDownload, RelayError, RelayHandle, RelayId, RelayMetadata, RelayService,
    RelayServiceError, RelayTokenHash,
};
use crate::review::{ReviewHandle, ReviewService, ReviewSession, Verdict};
use crate::runtime::Runtime;
use crate::settlement::{
    Balance, HoldKind, HoldRequest, SettlementGateway, SettlementHandle, SettlementService,
};
use crate::storage::{
    ArtifactLocator, AuthCredential, IdempotencyDecision, IdempotencyKey, IdempotencyOutcome,
    IdempotentOperation, StorageHandle, StorageService, TokenHash,
};
use crate::task::{TaskHandle, TaskService};
use crate::types::{AssignmentId, SessionId, TaskId, Timestamp};

use super::types::{
    AgentToken, AssignRequest, CreatedAssignment, CreatedHold, CreatedRelay, CreatedSession,
    PingResponse, RegisterAgentResponse, RequestedReview, ReviewRequest, ServerError,
    SubmittedArtifact, parse_assignment_id, parse_hold_id, parse_review_id, parse_session_id,
    parse_task_id, require_assignment_agent, require_task_publisher,
};

#[derive(Clone, Debug)]
pub struct PlatformApp {
    registry: RegistryHandle,
    heartbeat: HeartbeatHandle,
    tasks: TaskHandle,
    live_sessions: LiveSessionHandle,
    review: ReviewHandle,
    settlement: SettlementHandle,
    settlement_gateway: SettlementGateway,
    storage: StorageHandle,
    relay: RelayHandle,
    token_counter: Arc<AtomicU64>,
    registration_token: Option<String>,
}

impl PlatformApp {
    pub fn spawn() -> Result<Self, ServerError> {
        let registration_token = env::var("AGENT_MARKETPLACE_REGISTRATION_TOKEN")
            .ok()
            .and_then(non_empty_secret);
        Self::spawn_with_registration_token(registration_token)
    }

    pub fn spawn_with_registration_token(
        registration_token: Option<String>,
    ) -> Result<Self, ServerError> {
        let registry = RegistryService::spawn();
        let tasks = TaskService::spawn();
        let live_sessions = LiveSessionService::spawn();
        let review = ReviewService::spawn();
        let settlement = SettlementService::spawn();
        let storage = StorageService::spawn();
        let relay = RelayService::spawn();
        let runtime = Runtime::new(
            registry.clone(),
            settlement.clone(),
            live_sessions.clone(),
            tasks.clone(),
        );
        let heartbeat =
            HeartbeatService::spawn(HeartbeatConfig::default(), runtime.heartbeat_sink())?;
        let settlement_gateway =
            SettlementGateway::new(settlement.clone(), live_sessions.clone(), review.clone());

        Ok(Self {
            registry,
            heartbeat,
            tasks,
            live_sessions,
            review,
            settlement,
            settlement_gateway,
            storage,
            relay,
            token_counter: Arc::new(AtomicU64::new(0)),
            registration_token: registration_token.and_then(non_empty_secret),
        })
    }

    pub async fn shutdown(&self) {
        let _ = self.heartbeat.shutdown().await;
        let _ = self.registry.shutdown().await;
        let _ = self.tasks.shutdown().await;
        let _ = self.live_sessions.shutdown().await;
        let _ = self.review.shutdown().await;
        let _ = self.settlement.shutdown().await;
        let _ = self.storage.shutdown().await;
        let _ = self.relay.shutdown().await;
    }

    pub async fn register_agent(
        &self,
        identity: AgentIdentity,
        issued_at: Timestamp,
    ) -> Result<RegisterAgentResponse, ServerError> {
        self.register_agent_with_proof(identity, issued_at, None, None)
            .await
    }

    pub async fn register_agent_with_proof(
        &self,
        identity: AgentIdentity,
        issued_at: Timestamp,
        owner_token: Option<&AgentToken>,
        registration_token: Option<&str>,
    ) -> Result<RegisterAgentResponse, ServerError> {
        let agent_id = identity.agent_id.clone();
        self.require_registration_authority(&agent_id, owner_token, registration_token)
            .await?;
        let outcome = self
            .registry
            .register(identity)
            .await
            .map_err(|error| ServerError::component("registry", error))?;
        let token = self.issue_token(&agent_id, issued_at)?;
        self.storage
            .store_credential(AuthCredential::new(
                agent_id.clone(),
                hash_token(&token)?,
                issued_at,
            ))
            .await
            .map_err(|error| ServerError::component("storage", error))?;

        Ok(RegisterAgentResponse {
            agent_id,
            outcome,
            token,
        })
    }

    async fn require_registration_authority(
        &self,
        agent_id: &AgentId,
        owner_token: Option<&AgentToken>,
        registration_token: Option<&str>,
    ) -> Result<(), ServerError> {
        let admin_authorized = self.registration_token.as_deref().is_some_and(|expected| {
            registration_token.is_some_and(|actual| constant_time_eq(expected, actual))
        });
        let registered = self
            .registry
            .list_agents(ListAgentsQuery::new().include_deregistered(true))
            .await
            .map_err(|error| ServerError::component("registry", error))?
            .into_iter()
            .any(|agent| agent.agent_id == *agent_id);

        if registered {
            if admin_authorized {
                return Ok(());
            }
            let Some(owner_token) = owner_token else {
                return Err(ServerError::Forbidden {
                    agent_id: agent_id.clone(),
                    action: "re-register existing agent identity without owner proof",
                });
            };
            let caller = self.authenticate(owner_token).await?;
            if caller == *agent_id {
                return Ok(());
            }
            return Err(ServerError::Forbidden {
                agent_id: caller,
                action: "re-register another agent identity",
            });
        }

        if self.registration_token.is_some() && !admin_authorized {
            return Err(ServerError::Unauthorized);
        }

        Ok(())
    }

    pub async fn authenticate(&self, token: &AgentToken) -> Result<AgentId, ServerError> {
        self.storage
            .authenticate(hash_token(token)?)
            .await
            .map_err(|error| ServerError::component("storage", error))?
            .ok_or(ServerError::Unauthorized)
    }

    pub async fn declare_capabilities(
        &self,
        token: &AgentToken,
        capabilities: Vec<Capability>,
    ) -> Result<(), ServerError> {
        let agent_id = self.authenticate(token).await?;
        self.registry
            .declare_capabilities(agent_id, capabilities)
            .await
            .map_err(|error| ServerError::component("registry", error))?;
        Ok(())
    }

    pub async fn deregister(&self, token: &AgentToken, at: Timestamp) -> Result<bool, ServerError> {
        let agent_id = self.authenticate(token).await?;
        let deregistered = self
            .registry
            .deregister(agent_id)
            .await
            .map_err(|error| ServerError::component("registry", error))?;
        self.storage
            .revoke_credential(hash_token(token)?, at)
            .await
            .map_err(|error| ServerError::component("storage", error))?;
        Ok(deregistered)
    }

    pub async fn ping(&self, token: &AgentToken, busy: bool) -> Result<PingResponse, ServerError> {
        let agent_id = self.authenticate(token).await?;
        let outcome = self
            .heartbeat
            .ping(agent_id.clone(), busy)
            .await
            .map_err(|error| ServerError::component("heartbeat", error))?;
        self.registry
            .mark_alive(agent_id.clone())
            .await
            .map_err(|error| ServerError::component("registry", error))?;
        Ok(PingResponse { agent_id, outcome })
    }

    pub async fn discover(
        &self,
        query: DiscoveryQuery,
    ) -> Result<Vec<AgentCandidate>, ServerError> {
        self.registry
            .discover(query)
            .await
            .map_err(|error| ServerError::component("registry", error))
    }

    pub async fn list_agents(
        &self,
        query: ListAgentsQuery,
    ) -> Result<Vec<AgentListing>, ServerError> {
        self.registry
            .list_agents(query)
            .await
            .map_err(|error| ServerError::component("registry", error))
    }

    pub async fn create_task(
        &self,
        token: &AgentToken,
        key: IdempotencyKey,
        at: Timestamp,
    ) -> Result<TaskId, ServerError> {
        let caller = self.authenticate(token).await?;
        match self
            .begin_idempotency(
                caller.clone(),
                key.clone(),
                IdempotentOperation::CreateTask,
                at,
            )
            .await?
        {
            IdempotencyDecision::Replay(record) => replay_task(record.outcome),
            IdempotencyDecision::Started(_) => {
                let task_id = self
                    .tasks
                    .create(caller.clone(), at)
                    .await
                    .map_err(|error| ServerError::component("task", error))?;
                self.finish_idempotency(
                    caller,
                    key,
                    IdempotentOperation::CreateTask,
                    IdempotencyOutcome::Succeeded(format!("task:{task_id}")),
                    at,
                )
                .await?;
                Ok(task_id)
            }
        }
    }

    pub async fn add_participant(
        &self,
        token: &AgentToken,
        key: IdempotencyKey,
        task_id: TaskId,
        agent_id: AgentId,
        at: Timestamp,
    ) -> Result<(), ServerError> {
        let caller = self.authenticate(token).await?;
        self.require_publisher(&task_id, &caller, "add participant")
            .await?;
        match self
            .begin_idempotency(
                caller.clone(),
                key.clone(),
                IdempotentOperation::AddParticipant,
                at,
            )
            .await?
        {
            IdempotencyDecision::Replay(record) => replay_unit(record.outcome),
            IdempotencyDecision::Started(_) => {
                self.tasks
                    .add_participant(task_id, agent_id, at)
                    .await
                    .map_err(|error| ServerError::component("task", error))?;
                self.finish_idempotency(
                    caller,
                    key,
                    IdempotentOperation::AddParticipant,
                    IdempotencyOutcome::Succeeded("ok".to_string()),
                    at,
                )
                .await?;
                Ok(())
            }
        }
    }

    pub async fn create_session(
        &self,
        token: &AgentToken,
        key: IdempotencyKey,
        task_id: TaskId,
        at: Timestamp,
    ) -> Result<CreatedSession, ServerError> {
        let caller = self.authenticate(token).await?;
        self.require_publisher(&task_id, &caller, "create session")
            .await?;
        match self
            .begin_idempotency(
                caller.clone(),
                key.clone(),
                IdempotentOperation::CreateSession,
                at,
            )
            .await?
        {
            IdempotencyDecision::Replay(record) => Ok(CreatedSession {
                session_id: replay_session(record.outcome)?,
            }),
            IdempotencyDecision::Started(_) => {
                let session_id = self
                    .live_sessions
                    .create_session(task_id, at)
                    .await
                    .map_err(|error| ServerError::component("livesession", error))?;
                self.finish_idempotency(
                    caller,
                    key,
                    IdempotentOperation::CreateSession,
                    IdempotencyOutcome::Succeeded(format!("session:{session_id}")),
                    at,
                )
                .await?;
                Ok(CreatedSession { session_id })
            }
        }
    }

    pub async fn assign(
        &self,
        token: &AgentToken,
        key: IdempotencyKey,
        request: AssignRequest,
        at: Timestamp,
    ) -> Result<CreatedAssignment, ServerError> {
        let caller = self.authenticate(token).await?;
        self.require_publisher(&request.task_id, &caller, "create assignment")
            .await?;
        match self
            .begin_idempotency(
                caller.clone(),
                key.clone(),
                IdempotentOperation::CreateAssignment,
                at,
            )
            .await?
        {
            IdempotencyDecision::Replay(record) => {
                let assignment_id = replay_assignment(record.outcome)?;
                let assignment = self.get_assignment(assignment_id.clone()).await?;
                Ok(CreatedAssignment {
                    assignment_id,
                    kind: assignment.kind,
                })
            }
            IdempotencyDecision::Started(_) => {
                let assignment_id = self
                    .live_sessions
                    .assign(
                        request.task_id,
                        request.session_id,
                        request.agent_id,
                        request.kind.clone(),
                        at,
                    )
                    .await
                    .map_err(|error| ServerError::component("livesession", error))?;
                self.finish_idempotency(
                    caller,
                    key,
                    IdempotentOperation::CreateAssignment,
                    IdempotencyOutcome::Succeeded(format!("assignment:{assignment_id}")),
                    at,
                )
                .await?;
                Ok(CreatedAssignment {
                    assignment_id,
                    kind: request.kind,
                })
            }
        }
    }

    pub async fn assignments_for_self(
        &self,
        token: &AgentToken,
    ) -> Result<Vec<Assignment>, ServerError> {
        let caller = self.authenticate(token).await?;
        self.live_sessions
            .assignments_by_agent(caller)
            .await
            .map_err(|error| ServerError::component("livesession", error))
    }

    pub async fn get_assignment(
        &self,
        assignment_id: AssignmentId,
    ) -> Result<Assignment, ServerError> {
        self.live_sessions
            .get_assignment(assignment_id.clone())
            .await
            .map_err(|error| ServerError::component("livesession", error))?
            .ok_or_else(|| ServerError::NotFound(format!("assignment {assignment_id}")))
    }

    pub async fn review_assignments_for_target(
        &self,
        target_assignment_id: AssignmentId,
    ) -> Result<Vec<Assignment>, ServerError> {
        self.live_sessions
            .review_assignments_for_target(target_assignment_id)
            .await
            .map_err(|error| ServerError::component("livesession", error))
    }

    pub async fn submit_artifact(
        &self,
        token: &AgentToken,
        key: IdempotencyKey,
        assignment_id: AssignmentId,
        manifest: ArtifactManifest,
        manifest_uri: String,
        at: Timestamp,
    ) -> Result<SubmittedArtifact, ServerError> {
        let caller = self.authenticate(token).await?;
        let assignment = self.get_assignment(assignment_id.clone()).await?;
        require_assignment_agent(&assignment, &caller, "submit artifact")?;
        match self
            .begin_idempotency(
                caller.clone(),
                key.clone(),
                IdempotentOperation::SubmitArtifact,
                at,
            )
            .await?
        {
            IdempotencyDecision::Replay(record) => {
                let replayed_assignment_id = replay_assignment(record.outcome)?;
                self.submitted_artifact(replayed_assignment_id).await
            }
            IdempotencyDecision::Started(_) => {
                self.live_sessions
                    .submit_artifact(assignment_id.clone(), caller.clone(), manifest, at)
                    .await
                    .map_err(|error| ServerError::component("livesession", error))?;
                let submitted = self.get_assignment(assignment_id.clone()).await?;
                let manifest_hash = submitted
                    .output_hash
                    .clone()
                    .ok_or_else(|| ServerError::MissingAssignmentOutput(assignment_id.clone()))?;
                let locator = ArtifactLocator::new(
                    assignment_id.clone(),
                    manifest_hash.clone(),
                    manifest_uri,
                    caller.clone(),
                    at,
                )
                .map_err(|error| ServerError::component("storage", error))?;
                self.storage
                    .store_artifact_locator(locator.clone())
                    .await
                    .map_err(|error| ServerError::component("storage", error))?;
                self.finish_idempotency(
                    caller,
                    key,
                    IdempotentOperation::SubmitArtifact,
                    IdempotencyOutcome::Succeeded(format!("assignment:{assignment_id}")),
                    at,
                )
                .await?;
                Ok(SubmittedArtifact {
                    assignment_id,
                    manifest_hash,
                    locator,
                })
            }
        }
    }

    pub async fn get_artifact_locator(
        &self,
        assignment_id: AssignmentId,
    ) -> Result<ArtifactLocator, ServerError> {
        self.storage
            .get_artifact_locator(assignment_id.clone())
            .await
            .map_err(|error| ServerError::component("storage", error))?
            .ok_or_else(|| ServerError::NotFound(format!("artifact locator {assignment_id}")))
    }

    pub async fn request_review(
        &self,
        token: &AgentToken,
        key: IdempotencyKey,
        request: ReviewRequest,
        at: Timestamp,
    ) -> Result<RequestedReview, ServerError> {
        let caller = self.authenticate(token).await?;
        self.require_publisher(&request.task_id, &caller, "request review")
            .await?;
        self.validate_review_request(&request).await?;
        match self
            .begin_idempotency(
                caller.clone(),
                key.clone(),
                IdempotentOperation::RequestReview,
                at,
            )
            .await?
        {
            IdempotencyDecision::Replay(record) => Ok(RequestedReview {
                review_id: replay_review(record.outcome)?,
            }),
            IdempotencyDecision::Started(_) => {
                let review_id = self
                    .review
                    .request(
                        request.task_id,
                        request.target_assignment_id,
                        request.review_assignment_ids,
                        request.criteria,
                        at,
                    )
                    .await
                    .map_err(|error| ServerError::component("review", error))?;
                self.finish_idempotency(
                    caller,
                    key,
                    IdempotentOperation::RequestReview,
                    IdempotencyOutcome::Succeeded(format!("review:{review_id}")),
                    at,
                )
                .await?;
                Ok(RequestedReview { review_id })
            }
        }
    }

    pub async fn submit_review(
        &self,
        token: &AgentToken,
        key: IdempotencyKey,
        review_id: crate::review::ReviewId,
        review_assignment_id: AssignmentId,
        verdict: Verdict,
        at: Timestamp,
    ) -> Result<(), ServerError> {
        let caller = self.authenticate(token).await?;
        let assignment = self.get_assignment(review_assignment_id.clone()).await?;
        require_assignment_agent(&assignment, &caller, "submit review")?;
        if !matches!(assignment.kind, AssignmentKind::Review { .. }) {
            return Err(ServerError::InvalidAssignmentKind {
                assignment_id: review_assignment_id,
                expected: "review",
                actual: assignment.kind,
            });
        }
        match self
            .begin_idempotency(
                caller.clone(),
                key.clone(),
                IdempotentOperation::SubmitReview,
                at,
            )
            .await?
        {
            IdempotencyDecision::Replay(record) => {
                replay_unit(record.outcome)?;
                self.settle_after_review_submission_best_effort(
                    review_id.clone(),
                    review_assignment_id.clone(),
                    at,
                )
                .await;
                Ok(())
            }
            IdempotencyDecision::Started(_) => {
                let evidence = crate::review::ReviewArtifactEvidence::new(
                    assignment.assignment_id.clone(),
                    assignment.status,
                    assignment.output_hash,
                );
                self.review
                    .submit(review_id.clone(), evidence, verdict, at)
                    .await
                    .map_err(|error| ServerError::component("review", error))?;
                self.finish_idempotency(
                    caller,
                    key,
                    IdempotentOperation::SubmitReview,
                    IdempotencyOutcome::Succeeded("ok".to_string()),
                    at,
                )
                .await?;
                self.settle_after_review_submission_best_effort(
                    review_id,
                    review_assignment_id,
                    at,
                )
                .await;
                Ok(())
            }
        }
    }

    pub async fn collect_reviews_by_assignment(
        &self,
        assignment_id: AssignmentId,
    ) -> Result<Vec<ReviewSession>, ServerError> {
        self.review
            .collect_by_assignment(assignment_id)
            .await
            .map_err(|error| ServerError::component("review", error))
    }

    pub async fn deposit(
        &self,
        token: &AgentToken,
        key: IdempotencyKey,
        amount: u64,
        at: Timestamp,
    ) -> Result<(), ServerError> {
        let caller = self.authenticate(token).await?;
        match self
            .begin_idempotency(
                caller.clone(),
                key.clone(),
                IdempotentOperation::Deposit,
                at,
            )
            .await?
        {
            IdempotencyDecision::Replay(record) => replay_unit(record.outcome),
            IdempotencyDecision::Started(_) => {
                self.settlement
                    .deposit(caller.clone(), amount, at)
                    .await
                    .map_err(|error| ServerError::component("settlement", error))?;
                self.finish_idempotency(
                    caller,
                    key,
                    IdempotentOperation::Deposit,
                    IdempotencyOutcome::Succeeded("ok".to_string()),
                    at,
                )
                .await?;
                Ok(())
            }
        }
    }

    pub async fn hold(
        &self,
        token: &AgentToken,
        key: IdempotencyKey,
        request: HoldRequest,
        at: Timestamp,
    ) -> Result<CreatedHold, ServerError> {
        let caller = self.authenticate(token).await?;
        if request.from_agent != caller {
            return Err(ServerError::Forbidden {
                agent_id: caller,
                action: "hold funds for another payer",
            });
        }
        self.validate_hold_request(&request).await?;
        match self
            .begin_idempotency(
                request.from_agent.clone(),
                key.clone(),
                IdempotentOperation::Hold,
                at,
            )
            .await?
        {
            IdempotencyDecision::Replay(record) => Ok(CreatedHold {
                hold_id: replay_hold(record.outcome)?,
            }),
            IdempotencyDecision::Started(_) => {
                let caller = request.from_agent.clone();
                let hold_id = self
                    .settlement
                    .hold(request, at)
                    .await
                    .map_err(|error| ServerError::component("settlement", error))?;
                self.finish_idempotency(
                    caller,
                    key,
                    IdempotentOperation::Hold,
                    IdempotencyOutcome::Succeeded(format!("hold:{hold_id}")),
                    at,
                )
                .await?;
                Ok(CreatedHold { hold_id })
            }
        }
    }

    pub async fn release_execute_after_reviews(
        &self,
        token: &AgentToken,
        key: IdempotencyKey,
        hold_id: crate::settlement::HoldId,
        at: Timestamp,
    ) -> Result<(), ServerError> {
        let caller = self.authenticate(token).await?;
        self.require_hold_payer_or_publisher(&hold_id, &caller, "release execute hold")
            .await?;
        match self
            .begin_idempotency(
                caller.clone(),
                key.clone(),
                IdempotentOperation::ReleaseExecute,
                at,
            )
            .await?
        {
            IdempotencyDecision::Replay(record) => replay_unit(record.outcome),
            IdempotencyDecision::Started(_) => {
                self.settlement_gateway
                    .release_execute_after_reviews(hold_id, at)
                    .await
                    .map_err(|error| ServerError::component("settlement_gateway", error))?;
                self.finish_idempotency(
                    caller,
                    key,
                    IdempotentOperation::ReleaseExecute,
                    IdempotencyOutcome::Succeeded("ok".to_string()),
                    at,
                )
                .await?;
                Ok(())
            }
        }
    }

    pub async fn release_review_after_submission(
        &self,
        token: &AgentToken,
        key: IdempotencyKey,
        hold_id: crate::settlement::HoldId,
        review_id: crate::review::ReviewId,
        at: Timestamp,
    ) -> Result<(), ServerError> {
        let caller = self.authenticate(token).await?;
        self.require_hold_payer_or_publisher(&hold_id, &caller, "release review hold")
            .await?;
        match self
            .begin_idempotency(
                caller.clone(),
                key.clone(),
                IdempotentOperation::ReleaseReview,
                at,
            )
            .await?
        {
            IdempotencyDecision::Replay(record) => replay_unit(record.outcome),
            IdempotencyDecision::Started(_) => {
                self.settlement_gateway
                    .release_review_after_submission(hold_id, review_id, at)
                    .await
                    .map_err(|error| ServerError::component("settlement_gateway", error))?;
                self.finish_idempotency(
                    caller,
                    key,
                    IdempotentOperation::ReleaseReview,
                    IdempotencyOutcome::Succeeded("ok".to_string()),
                    at,
                )
                .await?;
                Ok(())
            }
        }
    }

    pub async fn refund(
        &self,
        token: &AgentToken,
        key: IdempotencyKey,
        hold_id: crate::settlement::HoldId,
        at: Timestamp,
    ) -> Result<(), ServerError> {
        let caller = self.authenticate(token).await?;
        self.require_hold_payer_or_publisher(&hold_id, &caller, "refund hold")
            .await?;
        match self
            .begin_idempotency(caller.clone(), key.clone(), IdempotentOperation::Refund, at)
            .await?
        {
            IdempotencyDecision::Replay(record) => replay_unit(record.outcome),
            IdempotencyDecision::Started(_) => {
                self.settlement
                    .refund(hold_id, at)
                    .await
                    .map_err(|error| ServerError::component("settlement", error))?;
                self.finish_idempotency(
                    caller,
                    key,
                    IdempotentOperation::Refund,
                    IdempotencyOutcome::Succeeded("ok".to_string()),
                    at,
                )
                .await?;
                Ok(())
            }
        }
    }

    pub async fn balance(&self, token: &AgentToken) -> Result<Balance, ServerError> {
        let caller = self.authenticate(token).await?;
        self.settlement
            .balance(caller)
            .await
            .map_err(|error| ServerError::component("settlement", error))
    }

    pub async fn create_relay_slot(
        &self,
        size_bytes: u64,
        ttl_secs: Option<u64>,
        max_downloads: Option<u32>,
        at: Timestamp,
    ) -> Result<CreatedRelay, ServerError> {
        let upload_token = issue_relay_token()?;
        let download_token = issue_relay_token()?;
        let created = self
            .relay
            .create_slot(
                size_bytes,
                ttl_secs,
                max_downloads,
                hash_relay_token(&upload_token)?,
                hash_relay_token(&download_token)?,
                at,
            )
            .await
            .map_err(map_relay_error)?;

        Ok(CreatedRelay {
            relay_id: created.relay_id,
            upload_token,
            download_token,
            expires_at: created.expires_at,
        })
    }

    pub async fn upload_relay_blob(
        &self,
        relay_id: RelayId,
        relay_token: &str,
        encrypted_blob: Vec<u8>,
        at: Timestamp,
    ) -> Result<RelayMetadata, ServerError> {
        self.relay
            .upload(relay_id, hash_relay_token(relay_token)?, encrypted_blob, at)
            .await
            .map_err(map_relay_error)
    }

    pub async fn download_relay_blob(
        &self,
        relay_id: RelayId,
        relay_token: &str,
        at: Timestamp,
    ) -> Result<RelayDownload, ServerError> {
        self.relay
            .download(relay_id, hash_relay_token(relay_token)?, at)
            .await
            .map_err(map_relay_error)
    }

    pub async fn delete_relay_blob(
        &self,
        relay_id: RelayId,
        relay_token: &str,
        at: Timestamp,
    ) -> Result<RelayMetadata, ServerError> {
        self.relay
            .delete(relay_id, hash_relay_token(relay_token)?, at)
            .await
            .map_err(map_relay_error)
    }

    fn issue_token(
        &self,
        agent_id: &AgentId,
        issued_at: Timestamp,
    ) -> Result<AgentToken, ServerError> {
        let sequence = self.token_counter.fetch_add(1, Ordering::Relaxed) + 1;
        let material = format!("{agent_id}:{}:{sequence}", issued_at.0);
        let digest = sha256_digest(material.as_bytes())
            .map_err(|error| ServerError::component("artifact", error))?;
        Ok(AgentToken::new(format!("agent-token-{}", digest.as_str())))
    }

    async fn require_publisher(
        &self,
        task_id: &TaskId,
        caller: &AgentId,
        action: &'static str,
    ) -> Result<(), ServerError> {
        let task = self
            .tasks
            .get(task_id.clone())
            .await
            .map_err(|error| ServerError::component("task", error))?
            .ok_or_else(|| ServerError::NotFound(format!("task {task_id}")))?;
        require_task_publisher(&task.publisher, caller, action)
    }

    async fn validate_hold_request(&self, request: &HoldRequest) -> Result<(), ServerError> {
        let assignment = self.get_assignment(request.assignment_id.clone()).await?;
        if assignment.task_id != request.task_id {
            return Err(ServerError::BadRequest(format!(
                "hold task {} does not match assignment {} task {}",
                request.task_id, request.assignment_id, assignment.task_id
            )));
        }
        if assignment.agent_id != request.agent_id {
            return Err(ServerError::BadRequest(format!(
                "hold payee {} does not match assignment {} agent {}",
                request.agent_id, request.assignment_id, assignment.agent_id
            )));
        }
        let kind_matches = matches!(
            (&request.kind, &assignment.kind),
            (HoldKind::Execute, AssignmentKind::Execute)
                | (HoldKind::Review, AssignmentKind::Review { .. })
        );
        if !kind_matches {
            return Err(ServerError::BadRequest(format!(
                "hold kind {:?} does not match assignment {} kind {:?}",
                request.kind, request.assignment_id, assignment.kind
            )));
        }

        Ok(())
    }

    async fn validate_review_request(&self, request: &ReviewRequest) -> Result<(), ServerError> {
        let target = self
            .get_assignment(request.target_assignment_id.clone())
            .await?;
        if target.task_id != request.task_id {
            return Err(ServerError::BadRequest(format!(
                "review target assignment {} belongs to task {}, not {}",
                request.target_assignment_id, target.task_id, request.task_id
            )));
        }
        if !matches!(&target.kind, AssignmentKind::Execute) {
            return Err(ServerError::InvalidAssignmentKind {
                assignment_id: target.assignment_id,
                expected: "execute",
                actual: target.kind.clone(),
            });
        }

        for review_assignment_id in &request.review_assignment_ids {
            let review_assignment = self.get_assignment(review_assignment_id.clone()).await?;
            if review_assignment.task_id != request.task_id {
                return Err(ServerError::BadRequest(format!(
                    "review assignment {review_assignment_id} belongs to task {}, not {}",
                    review_assignment.task_id, request.task_id
                )));
            }
            match &review_assignment.kind {
                AssignmentKind::Review {
                    target_assignment_id,
                } if target_assignment_id == &request.target_assignment_id => {}
                AssignmentKind::Review {
                    target_assignment_id,
                } => {
                    return Err(ServerError::BadRequest(format!(
                        "review assignment {review_assignment_id} targets {target_assignment_id}, not {}",
                        request.target_assignment_id
                    )));
                }
                actual => {
                    return Err(ServerError::InvalidAssignmentKind {
                        assignment_id: review_assignment.assignment_id,
                        expected: "review",
                        actual: actual.clone(),
                    });
                }
            }
        }

        Ok(())
    }

    async fn settle_after_review_submission_best_effort(
        &self,
        review_id: crate::review::ReviewId,
        review_assignment_id: AssignmentId,
        at: Timestamp,
    ) {
        if let Err(error) = self
            .settlement_gateway
            .settle_after_review_submission(review_id, review_assignment_id, at)
            .await
        {
            eprintln!("failed to auto-settle after review submission: {error}");
        }
    }

    async fn require_hold_payer_or_publisher(
        &self,
        hold_id: &crate::settlement::HoldId,
        caller: &AgentId,
        action: &'static str,
    ) -> Result<(), ServerError> {
        let hold = self
            .settlement
            .get_hold(hold_id.clone())
            .await
            .map_err(|error| ServerError::component("settlement", error))?
            .ok_or_else(|| ServerError::NotFound(format!("hold {hold_id}")))?;
        if hold.from_agent == *caller {
            return Ok(());
        }
        let task = self
            .tasks
            .get(hold.task_id.clone())
            .await
            .map_err(|error| ServerError::component("task", error))?
            .ok_or_else(|| ServerError::NotFound(format!("task {}", hold.task_id)))?;
        if task.publisher == *caller {
            return Ok(());
        }

        Err(ServerError::Forbidden {
            agent_id: caller.clone(),
            action,
        })
    }

    async fn submitted_artifact(
        &self,
        assignment_id: AssignmentId,
    ) -> Result<SubmittedArtifact, ServerError> {
        let assignment = self.get_assignment(assignment_id.clone()).await?;
        let manifest_hash = assignment
            .output_hash
            .ok_or_else(|| ServerError::MissingAssignmentOutput(assignment_id.clone()))?;
        let locator = self.get_artifact_locator(assignment_id.clone()).await?;
        Ok(SubmittedArtifact {
            assignment_id,
            manifest_hash,
            locator,
        })
    }

    async fn begin_idempotency(
        &self,
        caller: AgentId,
        key: IdempotencyKey,
        operation: IdempotentOperation,
        at: Timestamp,
    ) -> Result<IdempotencyDecision, ServerError> {
        self.storage
            .begin_idempotency(caller, key, operation, at)
            .await
            .map_err(|error| ServerError::component("storage", error))
    }

    async fn finish_idempotency(
        &self,
        caller: AgentId,
        key: IdempotencyKey,
        operation: IdempotentOperation,
        outcome: IdempotencyOutcome,
        at: Timestamp,
    ) -> Result<(), ServerError> {
        self.storage
            .finish_idempotency(caller, key, operation, outcome, at)
            .await
            .map_err(|error| ServerError::component("storage", error))?;
        Ok(())
    }
}

fn hash_token(token: &AgentToken) -> Result<TokenHash, ServerError> {
    let digest = sha256_digest(token.as_str().as_bytes())
        .map_err(|error| ServerError::component("artifact", error))?;
    Ok(TokenHash::from(digest.to_string()))
}

fn hash_relay_token(token: &str) -> Result<RelayTokenHash, ServerError> {
    if token.trim().is_empty() {
        return Err(ServerError::Unauthorized);
    }
    let digest = sha256_digest(token.as_bytes())
        .map_err(|error| ServerError::component("artifact", error))?;
    Ok(RelayTokenHash::from(digest.to_string()))
}

fn issue_relay_token() -> Result<String, ServerError> {
    let mut bytes = [0u8; 32];
    let mut random =
        File::open("/dev/urandom").map_err(|error| ServerError::component("relay", error))?;
    random
        .read_exact(&mut bytes)
        .map_err(|error| ServerError::component("relay", error))?;
    Ok(format!("relay-token-{}", hex_bytes(&bytes)))
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn map_relay_error(error: RelayServiceError) -> ServerError {
    match error {
        RelayServiceError::Relay(RelayError::Unauthorized) => ServerError::Unauthorized,
        RelayServiceError::Relay(
            RelayError::RelayNotFound(relay_id)
            | RelayError::Expired(relay_id)
            | RelayError::Deleted(relay_id),
        ) => ServerError::NotFound(format!("relay {relay_id}")),
        RelayServiceError::Relay(RelayError::InvalidConfig(message)) => {
            ServerError::component("relay", message)
        }
        RelayServiceError::Relay(error) => ServerError::BadRequest(error.to_string()),
        RelayServiceError::Stopped | RelayServiceError::ResponseDropped => {
            ServerError::component("relay", error)
        }
    }
}

fn non_empty_secret(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

fn constant_time_eq(expected: &str, actual: &str) -> bool {
    let expected = expected.as_bytes();
    let actual = actual.as_bytes();
    let mut diff = expected.len() ^ actual.len();
    for index in 0..expected.len().max(actual.len()) {
        let left = expected.get(index).copied().unwrap_or(0);
        let right = actual.get(index).copied().unwrap_or(0);
        diff |= usize::from(left ^ right);
    }
    diff == 0
}

fn replay_task(outcome: IdempotencyOutcome) -> Result<TaskId, ServerError> {
    match outcome {
        IdempotencyOutcome::Pending => Err(ServerError::IdempotencyInProgress),
        IdempotencyOutcome::Succeeded(value) => parse_task_id(value),
        IdempotencyOutcome::Failed(message) => Err(ServerError::component("idempotency", message)),
    }
}

fn replay_session(outcome: IdempotencyOutcome) -> Result<SessionId, ServerError> {
    match outcome {
        IdempotencyOutcome::Pending => Err(ServerError::IdempotencyInProgress),
        IdempotencyOutcome::Succeeded(value) => parse_session_id(value),
        IdempotencyOutcome::Failed(message) => Err(ServerError::component("idempotency", message)),
    }
}

fn replay_assignment(outcome: IdempotencyOutcome) -> Result<AssignmentId, ServerError> {
    match outcome {
        IdempotencyOutcome::Pending => Err(ServerError::IdempotencyInProgress),
        IdempotencyOutcome::Succeeded(value) => parse_assignment_id(value),
        IdempotencyOutcome::Failed(message) => Err(ServerError::component("idempotency", message)),
    }
}

fn replay_review(outcome: IdempotencyOutcome) -> Result<crate::review::ReviewId, ServerError> {
    match outcome {
        IdempotencyOutcome::Pending => Err(ServerError::IdempotencyInProgress),
        IdempotencyOutcome::Succeeded(value) => parse_review_id(value),
        IdempotencyOutcome::Failed(message) => Err(ServerError::component("idempotency", message)),
    }
}

fn replay_hold(outcome: IdempotencyOutcome) -> Result<crate::settlement::HoldId, ServerError> {
    match outcome {
        IdempotencyOutcome::Pending => Err(ServerError::IdempotencyInProgress),
        IdempotencyOutcome::Succeeded(value) => parse_hold_id(value),
        IdempotencyOutcome::Failed(message) => Err(ServerError::component("idempotency", message)),
    }
}

fn replay_unit(outcome: IdempotencyOutcome) -> Result<(), ServerError> {
    match outcome {
        IdempotencyOutcome::Pending => Err(ServerError::IdempotencyInProgress),
        IdempotencyOutcome::Succeeded(value) if value == "ok" => Ok(()),
        IdempotencyOutcome::Succeeded(value) => Err(ServerError::InvalidReplay {
            expected: "ok",
            actual: value,
        }),
        IdempotencyOutcome::Failed(message) => Err(ServerError::component("idempotency", message)),
    }
}

#[cfg(test)]
mod tests {
    use crate::artifact::{
        ArtifactFile, ArtifactKind, ArtifactManifest, HashDigest, seal_manifest,
    };
    use crate::registry::{AgentIdentity, Capability};
    use crate::review::{ReviewCriteria, Verdict, VerdictKind};
    use crate::settlement::{HoldKind, HoldRequest};

    use super::*;

    fn key(value: &str) -> IdempotencyKey {
        IdempotencyKey::from(value)
    }

    fn hash(value: u8) -> HashDigest {
        HashDigest::from_sha256_hex(format!("{value:064x}")).unwrap()
    }

    fn text_manifest(
        artifact_id: &str,
        task_id: &TaskId,
        assignment_id: &AssignmentId,
        producer_agent_id: &AgentId,
        at: Timestamp,
    ) -> ArtifactManifest {
        let file = ArtifactFile::new(
            format!("https://agent.example/{artifact_id}.md"),
            hash(1),
            "text/markdown",
            "text.markdown.utf8.v1",
            120,
        );
        seal_manifest(ArtifactManifest::new(
            artifact_id,
            task_id.clone(),
            assignment_id.clone(),
            producer_agent_id.clone(),
            ArtifactKind::Single,
            vec![file],
            at,
        ))
        .unwrap()
    }

    fn passed_verdict() -> Verdict {
        Verdict {
            kind: VerdictKind::Passed,
            score_bps: 10_000,
            feedback: "accepted".to_string(),
        }
    }

    async fn register(app: &PlatformApp, agent_id: &str, at: u64) -> AgentToken {
        app.register_agent(AgentIdentity::new(AgentId::from(agent_id)), Timestamp(at))
            .await
            .unwrap()
            .token
    }

    #[tokio::test]
    async fn duplicate_agent_registration_requires_owner_or_admin_proof() {
        let app = PlatformApp::spawn().unwrap();
        let first = app
            .register_agent(AgentIdentity::new(AgentId::from("agent-1")), Timestamp(1))
            .await
            .unwrap();

        let takeover = app
            .register_agent(AgentIdentity::new(AgentId::from("agent-1")), Timestamp(2))
            .await
            .unwrap_err();
        assert_eq!(
            takeover,
            ServerError::Forbidden {
                agent_id: AgentId::from("agent-1"),
                action: "re-register existing agent identity without owner proof",
            }
        );

        let updated = app
            .register_agent_with_proof(
                AgentIdentity::new(AgentId::from("agent-1")),
                Timestamp(3),
                Some(&first.token),
                None,
            )
            .await
            .unwrap();

        assert_eq!(updated.outcome, crate::registry::RegisterOutcome::Updated);
        app.shutdown().await;
    }

    #[tokio::test]
    async fn registration_token_can_protect_new_registration_and_admin_recovery() {
        let app =
            PlatformApp::spawn_with_registration_token(Some("invite-secret".to_string())).unwrap();

        assert_eq!(
            app.register_agent(AgentIdentity::new(AgentId::from("agent-1")), Timestamp(1))
                .await
                .unwrap_err(),
            ServerError::Unauthorized
        );

        app.register_agent_with_proof(
            AgentIdentity::new(AgentId::from("agent-1")),
            Timestamp(2),
            None,
            Some("invite-secret"),
        )
        .await
        .unwrap();
        let recovered = app
            .register_agent_with_proof(
                AgentIdentity::new(AgentId::from("agent-1")),
                Timestamp(3),
                None,
                Some("invite-secret"),
            )
            .await
            .unwrap();

        assert_eq!(recovered.outcome, crate::registry::RegisterOutcome::Updated);
        app.shutdown().await;
    }

    #[tokio::test]
    async fn app_coordinates_safe_agent_flow_and_gateway_settlement() {
        let app = PlatformApp::spawn().unwrap();
        let publisher_token = register(&app, "publisher", 1).await;
        let executor_token = register(&app, "executor", 2).await;
        let reviewer_token = register(&app, "reviewer", 3).await;

        app.declare_capabilities(&executor_token, vec![Capability::new("execute", 1)])
            .await
            .unwrap();
        app.declare_capabilities(&reviewer_token, vec![Capability::new("review", 1)])
            .await
            .unwrap();
        app.ping(&executor_token, false).await.unwrap();
        app.ping(&reviewer_token, false).await.unwrap();
        assert_eq!(
            app.discover(crate::registry::DiscoveryQuery::new("execute"))
                .await
                .unwrap()
                .len(),
            1
        );
        let listed_agents = app
            .list_agents(crate::registry::ListAgentsQuery::new())
            .await
            .unwrap();
        assert_eq!(listed_agents.len(), 3);
        assert!(listed_agents.iter().any(|agent| agent.alive));

        app.deposit(&publisher_token, key("deposit"), 130, Timestamp(4))
            .await
            .unwrap();
        let task_id = app
            .create_task(&publisher_token, key("task"), Timestamp(5))
            .await
            .unwrap();
        let replayed_task_id = app
            .create_task(&publisher_token, key("task"), Timestamp(6))
            .await
            .unwrap();
        assert_eq!(task_id, replayed_task_id);

        let session_id = app
            .create_session(
                &publisher_token,
                key("session"),
                task_id.clone(),
                Timestamp(7),
            )
            .await
            .unwrap()
            .session_id;
        app.add_participant(
            &publisher_token,
            key("participant-executor"),
            task_id.clone(),
            AgentId::from("executor"),
            Timestamp(8),
        )
        .await
        .unwrap();
        let execute_assignment = app
            .assign(
                &publisher_token,
                key("assign-execute"),
                AssignRequest::new(
                    task_id.clone(),
                    session_id.clone(),
                    AgentId::from("executor"),
                    AssignmentKind::Execute,
                ),
                Timestamp(9),
            )
            .await
            .unwrap()
            .assignment_id;
        let _execute_hold = app
            .hold(
                &publisher_token,
                key("hold-execute"),
                HoldRequest::new(
                    AgentId::from("publisher"),
                    100,
                    task_id.clone(),
                    execute_assignment.clone(),
                    AgentId::from("executor"),
                    HoldKind::Execute,
                ),
                Timestamp(10),
            )
            .await
            .unwrap()
            .hold_id;

        let forbidden = app
            .submit_artifact(
                &publisher_token,
                key("bad-submit"),
                execute_assignment.clone(),
                text_manifest(
                    "bad-output",
                    &task_id,
                    &execute_assignment,
                    &AgentId::from("executor"),
                    Timestamp(11),
                ),
                "https://publisher.example/bad.json".to_string(),
                Timestamp(11),
            )
            .await
            .unwrap_err();
        assert!(matches!(forbidden, ServerError::Forbidden { .. }));

        let submitted = app
            .submit_artifact(
                &executor_token,
                key("submit-execute"),
                execute_assignment.clone(),
                text_manifest(
                    "execute-output",
                    &task_id,
                    &execute_assignment,
                    &AgentId::from("executor"),
                    Timestamp(12),
                ),
                "https://executor.example/manifests/execute-output.json".to_string(),
                Timestamp(12),
            )
            .await
            .unwrap();
        assert_eq!(submitted.assignment_id, execute_assignment);
        assert_eq!(
            app.get_artifact_locator(execute_assignment.clone())
                .await
                .unwrap()
                .manifest_uri,
            "https://executor.example/manifests/execute-output.json"
        );

        app.add_participant(
            &publisher_token,
            key("participant-reviewer"),
            task_id.clone(),
            AgentId::from("reviewer"),
            Timestamp(13),
        )
        .await
        .unwrap();
        let review_assignment = app
            .assign(
                &publisher_token,
                key("assign-review"),
                AssignRequest::new(
                    task_id.clone(),
                    session_id,
                    AgentId::from("reviewer"),
                    AssignmentKind::Review {
                        target_assignment_id: execute_assignment.clone(),
                    },
                ),
                Timestamp(14),
            )
            .await
            .unwrap()
            .assignment_id;
        let _review_hold = app
            .hold(
                &publisher_token,
                key("hold-review"),
                HoldRequest::new(
                    AgentId::from("publisher"),
                    30,
                    task_id.clone(),
                    review_assignment.clone(),
                    AgentId::from("reviewer"),
                    HoldKind::Review,
                ),
                Timestamp(15),
            )
            .await
            .unwrap()
            .hold_id;
        let review_id = app
            .request_review(
                &publisher_token,
                key("request-review"),
                ReviewRequest::new(
                    task_id.clone(),
                    execute_assignment.clone(),
                    vec![review_assignment.clone()],
                    ReviewCriteria::plain_text("review output"),
                ),
                Timestamp(16),
            )
            .await
            .unwrap()
            .review_id;
        app.submit_artifact(
            &reviewer_token,
            key("submit-review-artifact"),
            review_assignment.clone(),
            text_manifest(
                "review-output",
                &task_id,
                &review_assignment,
                &AgentId::from("reviewer"),
                Timestamp(17),
            ),
            "https://reviewer.example/manifests/review-output.json".to_string(),
            Timestamp(17),
        )
        .await
        .unwrap();
        app.submit_review(
            &reviewer_token,
            key("submit-review"),
            review_id,
            review_assignment.clone(),
            passed_verdict(),
            Timestamp(18),
        )
        .await
        .unwrap();

        assert_eq!(app.balance(&publisher_token).await.unwrap(), 0);
        assert_eq!(app.balance(&executor_token).await.unwrap(), 100);
        assert_eq!(app.balance(&reviewer_token).await.unwrap(), 30);

        app.shutdown().await;
    }

    #[tokio::test]
    async fn hold_rejects_assignment_binding_mismatch() {
        let app = PlatformApp::spawn().unwrap();
        let publisher_token = register(&app, "publisher", 1).await;
        let _executor_token = register(&app, "executor", 2).await;
        let _other_token = register(&app, "other", 3).await;

        app.deposit(&publisher_token, key("deposit"), 100, Timestamp(4))
            .await
            .unwrap();
        let task_id = app
            .create_task(&publisher_token, key("task"), Timestamp(5))
            .await
            .unwrap();
        let session_id = app
            .create_session(
                &publisher_token,
                key("session"),
                task_id.clone(),
                Timestamp(6),
            )
            .await
            .unwrap()
            .session_id;
        let assignment_id = app
            .assign(
                &publisher_token,
                key("assign"),
                AssignRequest::new(
                    task_id.clone(),
                    session_id,
                    AgentId::from("executor"),
                    AssignmentKind::Execute,
                ),
                Timestamp(7),
            )
            .await
            .unwrap()
            .assignment_id;

        let error = app
            .hold(
                &publisher_token,
                key("bad-hold"),
                HoldRequest::new(
                    AgentId::from("publisher"),
                    100,
                    task_id,
                    assignment_id,
                    AgentId::from("other"),
                    HoldKind::Execute,
                ),
                Timestamp(8),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, ServerError::BadRequest(message) if message.contains("payee")));
        assert_eq!(app.balance(&publisher_token).await.unwrap(), 100);

        app.shutdown().await;
    }

    #[tokio::test]
    async fn request_review_rejects_review_assignment_for_different_target() {
        let app = PlatformApp::spawn().unwrap();
        let publisher_token = register(&app, "publisher", 1).await;

        let task_id = app
            .create_task(&publisher_token, key("task"), Timestamp(2))
            .await
            .unwrap();
        let session_id = app
            .create_session(
                &publisher_token,
                key("session"),
                task_id.clone(),
                Timestamp(3),
            )
            .await
            .unwrap()
            .session_id;
        let first_execute = app
            .assign(
                &publisher_token,
                key("assign-execute-1"),
                AssignRequest::new(
                    task_id.clone(),
                    session_id.clone(),
                    AgentId::from("executor-1"),
                    AssignmentKind::Execute,
                ),
                Timestamp(4),
            )
            .await
            .unwrap()
            .assignment_id;
        let second_execute = app
            .assign(
                &publisher_token,
                key("assign-execute-2"),
                AssignRequest::new(
                    task_id.clone(),
                    session_id.clone(),
                    AgentId::from("executor-2"),
                    AssignmentKind::Execute,
                ),
                Timestamp(5),
            )
            .await
            .unwrap()
            .assignment_id;
        let review_assignment = app
            .assign(
                &publisher_token,
                key("assign-review"),
                AssignRequest::new(
                    task_id.clone(),
                    session_id,
                    AgentId::from("reviewer"),
                    AssignmentKind::Review {
                        target_assignment_id: second_execute,
                    },
                ),
                Timestamp(6),
            )
            .await
            .unwrap()
            .assignment_id;

        let error = app
            .request_review(
                &publisher_token,
                key("bad-review"),
                ReviewRequest::new(
                    task_id,
                    first_execute,
                    vec![review_assignment],
                    ReviewCriteria::plain_text("review wrong target"),
                ),
                Timestamp(7),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, ServerError::BadRequest(message) if message.contains("targets")));

        app.shutdown().await;
    }

    #[tokio::test]
    async fn revoked_token_cannot_call_authenticated_methods() {
        let app = PlatformApp::spawn().unwrap();
        let token = register(&app, "agent-1", 1).await;

        assert!(app.deregister(&token, Timestamp(2)).await.unwrap());
        assert_eq!(
            app.authenticate(&token).await.unwrap_err(),
            ServerError::Unauthorized
        );

        app.shutdown().await;
    }
}
