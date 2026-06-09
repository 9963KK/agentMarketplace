use std::collections::HashSet;
use std::error::Error;
use std::fmt;

use crate::livesession::{
    AssignmentKind, AssignmentStatus, LiveSessionHandle, LiveSessionServiceError,
};
use crate::review::{ReviewHandle, ReviewId, ReviewServiceError, ReviewSession, VerdictKind};
use crate::types::{AssignmentId, Timestamp};

use super::service::{SettlementHandle, SettlementServiceError};
use super::types::{HoldId, HoldKind, ReleaseEvidence};

#[derive(Clone, Debug)]
pub struct SettlementGateway {
    settlement: SettlementHandle,
    live_sessions: LiveSessionHandle,
    review: ReviewHandle,
}

impl SettlementGateway {
    pub fn new(
        settlement: SettlementHandle,
        live_sessions: LiveSessionHandle,
        review: ReviewHandle,
    ) -> Self {
        Self {
            settlement,
            live_sessions,
            review,
        }
    }

    pub async fn release_execute_after_reviews(
        &self,
        hold_id: impl Into<HoldId>,
        at: Timestamp,
    ) -> Result<(), SettlementGatewayError> {
        let hold_id = hold_id.into();
        let hold = self
            .settlement
            .get_hold(hold_id.clone())
            .await
            .map_err(SettlementGatewayError::Settlement)?
            .ok_or_else(|| SettlementGatewayError::HoldNotFound(hold_id.clone()))?;
        if hold.kind != HoldKind::Execute {
            return Err(SettlementGatewayError::HoldKindMismatch {
                hold_id,
                expected: HoldKind::Execute,
                actual: hold.kind,
            });
        }

        let assignment = self
            .live_sessions
            .get_assignment(hold.assignment_id.clone())
            .await
            .map_err(SettlementGatewayError::LiveSession)?
            .ok_or_else(|| {
                SettlementGatewayError::AssignmentNotFound(hold.assignment_id.clone())
            })?;
        if assignment.kind != AssignmentKind::Execute {
            return Err(SettlementGatewayError::AssignmentKindMismatch {
                assignment_id: assignment.assignment_id,
                expected: AssignmentKind::Execute,
                actual: assignment.kind,
            });
        }
        if !matches!(
            assignment.status,
            AssignmentStatus::Submitted | AssignmentStatus::Approved
        ) {
            return Err(SettlementGatewayError::ExecuteAssignmentNotSubmitted {
                assignment_id: assignment.assignment_id,
                status: assignment.status,
            });
        }

        let review_assignments = self
            .live_sessions
            .review_assignments_for_target(hold.assignment_id.clone())
            .await
            .map_err(SettlementGatewayError::LiveSession)?;
        if review_assignments.is_empty() {
            return Err(SettlementGatewayError::NoReviewAssignments {
                assignment_id: hold.assignment_id,
            });
        }

        let review_assignment_ids = review_assignments
            .iter()
            .map(|assignment| assignment.assignment_id.clone())
            .collect::<HashSet<_>>();
        let review_sessions = self
            .review
            .collect_by_assignment(hold.assignment_id.clone())
            .await
            .map_err(SettlementGatewayError::Review)?;
        let latest_review = latest_review_session(&review_sessions).ok_or_else(|| {
            SettlementGatewayError::NoReviewSession {
                assignment_id: hold.assignment_id.clone(),
            }
        })?;
        validate_review_session_passed(latest_review, &review_assignment_ids, &review_assignments)?;

        self.settlement
            .release(
                hold.hold_id,
                ReleaseEvidence::AssignmentOutputAccepted {
                    task_id: hold.task_id,
                    assignment_id: hold.assignment_id,
                    review_ids: vec![latest_review.review_id.clone()],
                },
                at,
            )
            .await
            .map_err(SettlementGatewayError::Settlement)
    }
}

fn latest_review_session(review_sessions: &[ReviewSession]) -> Option<&ReviewSession> {
    review_sessions.iter().max_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.review_id.cmp(&right.review_id))
    })
}

fn validate_review_session_passed(
    review_session: &ReviewSession,
    live_review_assignment_ids: &HashSet<AssignmentId>,
    review_assignments: &[crate::livesession::Assignment],
) -> Result<(), SettlementGatewayError> {
    if review_session.review_assignment_ids.is_empty() {
        return Err(SettlementGatewayError::NoReviewAssignments {
            assignment_id: review_session.target_assignment_id.clone(),
        });
    }

    for review_assignment_id in &review_session.review_assignment_ids {
        if !live_review_assignment_ids.contains(review_assignment_id) {
            return Err(SettlementGatewayError::ReviewAssignmentNotRegistered {
                review_id: review_session.review_id.clone(),
                review_assignment_id: review_assignment_id.clone(),
            });
        }

        let review_assignment = review_assignments
            .iter()
            .find(|assignment| assignment.assignment_id == *review_assignment_id)
            .expect("review assignment id came from live session index");
        if !matches!(
            review_assignment.status,
            AssignmentStatus::Submitted | AssignmentStatus::Approved
        ) {
            return Err(SettlementGatewayError::ReviewAssignmentNotSubmitted {
                assignment_id: review_assignment.assignment_id.clone(),
                status: review_assignment.status,
            });
        }

        let Some(verdict) = review_session
            .verdicts
            .iter()
            .find(|verdict| verdict.review_assignment_id == *review_assignment_id)
        else {
            return Err(SettlementGatewayError::ReviewVerdictMissing {
                review_id: review_session.review_id.clone(),
                review_assignment_id: review_assignment_id.clone(),
            });
        };
        if verdict.verdict.kind != VerdictKind::Passed {
            return Err(SettlementGatewayError::ReviewVerdictNotPassed {
                review_id: review_session.review_id.clone(),
                review_assignment_id: review_assignment_id.clone(),
                kind: verdict.verdict.kind.clone(),
            });
        }
    }

    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettlementGatewayError {
    Settlement(SettlementServiceError),
    LiveSession(LiveSessionServiceError),
    Review(ReviewServiceError),
    HoldNotFound(HoldId),
    AssignmentNotFound(AssignmentId),
    HoldKindMismatch {
        hold_id: HoldId,
        expected: HoldKind,
        actual: HoldKind,
    },
    AssignmentKindMismatch {
        assignment_id: AssignmentId,
        expected: AssignmentKind,
        actual: AssignmentKind,
    },
    ExecuteAssignmentNotSubmitted {
        assignment_id: AssignmentId,
        status: AssignmentStatus,
    },
    NoReviewAssignments {
        assignment_id: AssignmentId,
    },
    NoReviewSession {
        assignment_id: AssignmentId,
    },
    ReviewAssignmentNotRegistered {
        review_id: ReviewId,
        review_assignment_id: AssignmentId,
    },
    ReviewAssignmentNotSubmitted {
        assignment_id: AssignmentId,
        status: AssignmentStatus,
    },
    ReviewVerdictMissing {
        review_id: ReviewId,
        review_assignment_id: AssignmentId,
    },
    ReviewVerdictNotPassed {
        review_id: ReviewId,
        review_assignment_id: AssignmentId,
        kind: VerdictKind,
    },
}

impl fmt::Display for SettlementGatewayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettlementGatewayError::Settlement(error) => write!(f, "{error}"),
            SettlementGatewayError::LiveSession(error) => write!(f, "{error}"),
            SettlementGatewayError::Review(error) => write!(f, "{error}"),
            SettlementGatewayError::HoldNotFound(hold_id) => {
                write!(f, "hold not found for settlement gateway: {hold_id}")
            }
            SettlementGatewayError::AssignmentNotFound(assignment_id) => {
                write!(
                    f,
                    "assignment not found for settlement gateway: {assignment_id}"
                )
            }
            SettlementGatewayError::HoldKindMismatch {
                hold_id,
                expected,
                actual,
            } => write!(
                f,
                "hold kind mismatch for settlement gateway {hold_id}: expected={expected:?}, actual={actual:?}"
            ),
            SettlementGatewayError::AssignmentKindMismatch {
                assignment_id,
                expected,
                actual,
            } => write!(
                f,
                "assignment kind mismatch for settlement gateway {assignment_id}: expected={expected:?}, actual={actual:?}"
            ),
            SettlementGatewayError::ExecuteAssignmentNotSubmitted {
                assignment_id,
                status,
            } => write!(
                f,
                "execute assignment is not submitted for release: {assignment_id}, status={status:?}"
            ),
            SettlementGatewayError::NoReviewAssignments { assignment_id } => {
                write!(
                    f,
                    "execute assignment has no review assignments: {assignment_id}"
                )
            }
            SettlementGatewayError::NoReviewSession { assignment_id } => {
                write!(
                    f,
                    "execute assignment has no review session: {assignment_id}"
                )
            }
            SettlementGatewayError::ReviewAssignmentNotRegistered {
                review_id,
                review_assignment_id,
            } => write!(
                f,
                "review {review_id} references an assignment not registered in live session: {review_assignment_id}"
            ),
            SettlementGatewayError::ReviewAssignmentNotSubmitted {
                assignment_id,
                status,
            } => write!(
                f,
                "review assignment is not submitted for release: {assignment_id}, status={status:?}"
            ),
            SettlementGatewayError::ReviewVerdictMissing {
                review_id,
                review_assignment_id,
            } => write!(
                f,
                "review {review_id} is missing verdict from review assignment {review_assignment_id}"
            ),
            SettlementGatewayError::ReviewVerdictNotPassed {
                review_id,
                review_assignment_id,
                kind,
            } => write!(
                f,
                "review {review_id} verdict is not passed for review assignment {review_assignment_id}: kind={kind:?}"
            ),
        }
    }
}

impl Error for SettlementGatewayError {}

#[cfg(test)]
mod tests {
    use crate::heartbeat::AgentId;
    use crate::livesession::{AssignmentKind, LiveSessionService};
    use crate::review::{ReviewArtifactEvidence, ReviewCriteria, ReviewService, Verdict};
    use crate::settlement::{HoldRequest, SettlementService};
    use crate::task::TaskService;
    use crate::types::TaskId;

    use super::*;

    fn passed_verdict() -> Verdict {
        Verdict {
            kind: VerdictKind::Passed,
            score_bps: 10_000,
            feedback: "accepted".to_string(),
        }
    }

    fn failed_verdict() -> Verdict {
        Verdict {
            kind: VerdictKind::Failed,
            score_bps: 0,
            feedback: "rejected".to_string(),
        }
    }

    async fn submitted_artifact_evidence(
        live_sessions: &LiveSessionHandle,
        assignment_id: AssignmentId,
    ) -> ReviewArtifactEvidence {
        let assignment = live_sessions
            .get_assignment(assignment_id)
            .await
            .unwrap()
            .unwrap();
        ReviewArtifactEvidence::new(
            assignment.assignment_id,
            assignment.status,
            assignment.output_hash,
        )
    }

    async fn setup_reviewed_execute(
        verdict: Verdict,
    ) -> (
        SettlementGateway,
        SettlementHandle,
        LiveSessionHandle,
        ReviewHandle,
        HoldId,
        AssignmentId,
        AssignmentId,
    ) {
        let settlement = SettlementService::spawn();
        let live_sessions = LiveSessionService::spawn();
        let review = ReviewService::spawn();
        let tasks = TaskService::spawn();
        let gateway =
            SettlementGateway::new(settlement.clone(), live_sessions.clone(), review.clone());

        let publisher = AgentId::from("publisher");
        let executor = AgentId::from("executor");
        let reviewer = AgentId::from("reviewer");
        let task_id = tasks.create(publisher.clone(), Timestamp(1)).await.unwrap();
        let session_id = live_sessions
            .create_session(task_id.clone(), Timestamp(2))
            .await
            .unwrap();
        let execute_assignment = live_sessions
            .assign(
                task_id.clone(),
                session_id.clone(),
                executor.clone(),
                AssignmentKind::Execute,
                Timestamp(3),
            )
            .await
            .unwrap();
        settlement
            .deposit(publisher.clone(), 100, Timestamp(4))
            .await
            .unwrap();
        let hold_id = settlement
            .hold(
                HoldRequest::new(
                    publisher,
                    100,
                    task_id.clone(),
                    execute_assignment.clone(),
                    executor.clone(),
                    HoldKind::Execute,
                ),
                Timestamp(5),
            )
            .await
            .unwrap();
        live_sessions
            .submit_output(
                execute_assignment.clone(),
                executor,
                "execute-output",
                Timestamp(6),
            )
            .await
            .unwrap();

        let review_assignment = live_sessions
            .assign(
                task_id.clone(),
                session_id,
                reviewer.clone(),
                AssignmentKind::Review {
                    target_assignment_id: execute_assignment.clone(),
                },
                Timestamp(7),
            )
            .await
            .unwrap();
        live_sessions
            .submit_output(
                review_assignment.clone(),
                reviewer,
                "review-output",
                Timestamp(8),
            )
            .await
            .unwrap();
        let review_id = review
            .request(
                task_id,
                execute_assignment.clone(),
                vec![review_assignment.clone()],
                ReviewCriteria::plain_text("review execute output"),
                Timestamp(9),
            )
            .await
            .unwrap();
        let evidence = submitted_artifact_evidence(&live_sessions, review_assignment.clone()).await;
        review
            .submit(review_id, evidence, verdict, Timestamp(10))
            .await
            .unwrap();

        let _ = tasks.shutdown().await;

        (
            gateway,
            settlement,
            live_sessions,
            review,
            hold_id,
            execute_assignment,
            review_assignment,
        )
    }

    #[tokio::test]
    async fn gateway_releases_execute_hold_after_latest_review_passes() {
        let (gateway, settlement, live_sessions, review, hold_id, _, _) =
            setup_reviewed_execute(passed_verdict()).await;

        gateway
            .release_execute_after_reviews(hold_id.clone(), Timestamp(11))
            .await
            .unwrap();

        assert_eq!(settlement.balance("executor").await.unwrap(), 100);
        settlement.shutdown().await.unwrap();
        live_sessions.shutdown().await.unwrap();
        review.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn gateway_rejects_execute_release_without_review_assignment() {
        let settlement = SettlementService::spawn();
        let live_sessions = LiveSessionService::spawn();
        let review = ReviewService::spawn();
        let gateway =
            SettlementGateway::new(settlement.clone(), live_sessions.clone(), review.clone());

        let task_id = TaskId::from("task-1");
        let session_id = live_sessions
            .create_session(task_id.clone(), Timestamp(1))
            .await
            .unwrap();
        let execute_assignment = live_sessions
            .assign(
                task_id.clone(),
                session_id,
                "executor",
                AssignmentKind::Execute,
                Timestamp(2),
            )
            .await
            .unwrap();
        settlement
            .deposit("publisher", 100, Timestamp(3))
            .await
            .unwrap();
        let hold_id = settlement
            .hold(
                HoldRequest::new(
                    "publisher",
                    100,
                    task_id,
                    execute_assignment.clone(),
                    "executor",
                    HoldKind::Execute,
                ),
                Timestamp(4),
            )
            .await
            .unwrap();
        live_sessions
            .submit_output(
                execute_assignment.clone(),
                "executor",
                "execute-output",
                Timestamp(5),
            )
            .await
            .unwrap();

        assert_eq!(
            gateway
                .release_execute_after_reviews(hold_id, Timestamp(6))
                .await
                .unwrap_err(),
            SettlementGatewayError::NoReviewAssignments {
                assignment_id: execute_assignment
            }
        );

        settlement.shutdown().await.unwrap();
        live_sessions.shutdown().await.unwrap();
        review.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn gateway_rejects_execute_release_when_latest_review_fails() {
        let (gateway, settlement, live_sessions, review, hold_id, _, review_assignment) =
            setup_reviewed_execute(failed_verdict()).await;

        let error = gateway
            .release_execute_after_reviews(hold_id, Timestamp(11))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            SettlementGatewayError::ReviewVerdictNotPassed {
                review_assignment_id,
                kind: VerdictKind::Failed,
                ..
            } if review_assignment_id == review_assignment
        ));
        assert_eq!(settlement.balance("executor").await.unwrap(), 0);
        settlement.shutdown().await.unwrap();
        live_sessions.shutdown().await.unwrap();
        review.shutdown().await.unwrap();
    }
}
