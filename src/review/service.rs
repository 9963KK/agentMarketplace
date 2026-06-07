use std::error::Error;
use std::fmt;

use tokio::sync::{mpsc, oneshot};

use crate::chain::{ArtifactRef, NodeId, Timestamp};
use crate::heartbeat::AgentId;

use super::ReviewCore;
use super::types::{ReviewCriteria, ReviewError, ReviewId, ReviewSession, Verdict, VerdictRecord};

const DEFAULT_COMMAND_BUFFER: usize = 128;

#[derive(Debug)]
pub enum ReviewCommand {
    Request {
        node_id: NodeId,
        artifact_ref: ArtifactRef,
        allowed_reviewers: Vec<AgentId>,
        criteria: ReviewCriteria,
        created_at: Timestamp,
        reply: oneshot::Sender<Result<ReviewId, ReviewError>>,
    },
    Submit {
        review_id: ReviewId,
        reviewer_id: AgentId,
        verdict: Verdict,
        submitted_at: Timestamp,
        reply: oneshot::Sender<Result<(), ReviewError>>,
    },
    Collect {
        review_id: ReviewId,
        reply: oneshot::Sender<Option<Vec<VerdictRecord>>>,
    },
    CollectByNode {
        node_id: NodeId,
        reply: oneshot::Sender<Vec<ReviewSession>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

#[derive(Clone, Debug)]
pub struct ReviewHandle {
    commands: mpsc::Sender<ReviewCommand>,
}

impl ReviewHandle {
    pub async fn request(
        &self,
        node_id: impl Into<NodeId>,
        artifact_ref: ArtifactRef,
        allowed_reviewers: Vec<AgentId>,
        criteria: ReviewCriteria,
        created_at: Timestamp,
    ) -> Result<ReviewId, ReviewServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ReviewCommand::Request {
            node_id: node_id.into(),
            artifact_ref,
            allowed_reviewers,
            criteria,
            created_at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| ReviewServiceError::ResponseDropped)?
            .map_err(ReviewServiceError::Review)
    }

    pub async fn submit(
        &self,
        review_id: impl Into<ReviewId>,
        reviewer_id: impl Into<AgentId>,
        verdict: Verdict,
        submitted_at: Timestamp,
    ) -> Result<(), ReviewServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ReviewCommand::Submit {
            review_id: review_id.into(),
            reviewer_id: reviewer_id.into(),
            verdict,
            submitted_at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| ReviewServiceError::ResponseDropped)?
            .map_err(ReviewServiceError::Review)
    }

    pub async fn collect(
        &self,
        review_id: impl Into<ReviewId>,
    ) -> Result<Option<Vec<VerdictRecord>>, ReviewServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ReviewCommand::Collect {
            review_id: review_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| ReviewServiceError::ResponseDropped)
    }

    pub async fn collect_by_node(
        &self,
        node_id: impl Into<NodeId>,
    ) -> Result<Vec<ReviewSession>, ReviewServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ReviewCommand::CollectByNode {
            node_id: node_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| ReviewServiceError::ResponseDropped)
    }

    pub async fn shutdown(&self) -> Result<(), ReviewServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ReviewCommand::Shutdown { reply }).await?;
        response
            .await
            .map_err(|_| ReviewServiceError::ResponseDropped)
    }

    async fn send(&self, command: ReviewCommand) -> Result<(), ReviewServiceError> {
        self.commands
            .send(command)
            .await
            .map_err(|_| ReviewServiceError::Stopped)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewServiceError {
    Review(ReviewError),
    Stopped,
    ResponseDropped,
}

impl fmt::Display for ReviewServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReviewServiceError::Review(error) => write!(f, "{error}"),
            ReviewServiceError::Stopped => f.write_str("review service is stopped"),
            ReviewServiceError::ResponseDropped => {
                f.write_str("review service dropped the response")
            }
        }
    }
}

impl Error for ReviewServiceError {}

pub struct ReviewService {
    core: ReviewCore,
    commands: mpsc::Receiver<ReviewCommand>,
}

impl ReviewService {
    pub fn spawn() -> ReviewHandle {
        Self::spawn_with_buffer(DEFAULT_COMMAND_BUFFER)
    }

    pub fn spawn_with_buffer(command_buffer: usize) -> ReviewHandle {
        assert!(
            command_buffer > 0,
            "review command buffer must be greater than zero"
        );
        let (commands, receiver) = mpsc::channel(command_buffer);
        let service = Self {
            core: ReviewCore::new(),
            commands: receiver,
        };

        tokio::spawn(service.run());

        ReviewHandle { commands }
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

    fn handle_command(&mut self, command: ReviewCommand) -> Option<oneshot::Sender<()>> {
        match command {
            ReviewCommand::Request {
                node_id,
                artifact_ref,
                allowed_reviewers,
                criteria,
                created_at,
                reply,
            } => {
                let _ = reply.send(self.core.request(
                    node_id,
                    artifact_ref,
                    allowed_reviewers,
                    criteria,
                    created_at,
                ));
                None
            }
            ReviewCommand::Submit {
                review_id,
                reviewer_id,
                verdict,
                submitted_at,
                reply,
            } => {
                let _ =
                    reply.send(
                        self.core
                            .submit(&review_id, reviewer_id, verdict, submitted_at),
                    );
                None
            }
            ReviewCommand::Collect { review_id, reply } => {
                let _ = reply.send(self.core.collect(&review_id));
                None
            }
            ReviewCommand::CollectByNode { node_id, reply } => {
                let _ = reply.send(self.core.collect_by_node(&node_id));
                None
            }
            ReviewCommand::Shutdown { reply } => Some(reply),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::chain::{ArtifactId, Hash};
    use crate::review::{MAX_CRITERIA_BYTES, VerdictKind};

    use super::*;

    fn agent(id: &str) -> AgentId {
        AgentId::from(id)
    }

    fn artifact_ref(id: &str, root_hash: &str) -> ArtifactRef {
        ArtifactRef {
            artifact_id: ArtifactId::from(id),
            root_hash: Hash::from(root_hash),
        }
    }

    fn verdict(kind: VerdictKind, score_bps: u16) -> Verdict {
        Verdict {
            kind,
            score_bps,
            feedback: "reviewed".to_string(),
        }
    }

    #[tokio::test]
    async fn service_requests_submits_and_collects_verdicts() {
        let review = ReviewService::spawn();

        let review_id = review
            .request(
                "node-1",
                artifact_ref("artifact-1", "hash-1"),
                vec![agent("reviewer-1")],
                ReviewCriteria::plain_text("check output"),
                Timestamp(1),
            )
            .await
            .unwrap();
        review
            .submit(
                review_id.clone(),
                "reviewer-1",
                verdict(VerdictKind::Passed, 9_000),
                Timestamp(2),
            )
            .await
            .unwrap();

        let verdicts = review.collect(review_id.clone()).await.unwrap().unwrap();
        let by_node = review.collect_by_node("node-1").await.unwrap();

        assert_eq!(verdicts.len(), 1);
        assert_eq!(by_node.len(), 1);
        assert_eq!(by_node[0].review_id, review_id);

        review.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_returns_review_errors() {
        let review = ReviewService::spawn();

        let error = review
            .request(
                "node-1",
                artifact_ref("artifact-1", "hash-1"),
                vec![agent("reviewer-1")],
                ReviewCriteria::plain_text("x".repeat(MAX_CRITERIA_BYTES + 1)),
                Timestamp(1),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error,
            ReviewServiceError::Review(ReviewError::CriteriaTooLarge {
                max_bytes: MAX_CRITERIA_BYTES,
                actual_bytes: MAX_CRITERIA_BYTES + 1,
            })
        );

        review.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_stops_after_shutdown() {
        let review = ReviewService::spawn();

        review.shutdown().await.unwrap();

        assert_eq!(
            review
                .request(
                    "node-1",
                    artifact_ref("artifact-1", "hash-1"),
                    Vec::new(),
                    ReviewCriteria::plain_text("check output"),
                    Timestamp(1),
                )
                .await
                .unwrap_err(),
            ReviewServiceError::Stopped
        );
    }
}
