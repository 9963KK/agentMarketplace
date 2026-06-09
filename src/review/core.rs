use std::collections::{HashMap, HashSet};

use crate::types::{AssignmentId, TaskId, Timestamp};

use crate::livesession::AssignmentStatus;

use super::types::{
    ReviewArtifactEvidence, ReviewCriteria, ReviewError, ReviewId, ReviewSession, Verdict,
    VerdictRecord,
};

pub const MAX_CRITERIA_BYTES: usize = 16 * 1024;
pub const MAX_FEEDBACK_BYTES: usize = 32 * 1024;
const MAX_SCORE_BPS: u16 = 10_000;

#[derive(Debug, Default)]
pub struct ReviewCore {
    sessions: HashMap<ReviewId, ReviewSession>,
    sessions_by_task: HashMap<TaskId, Vec<ReviewId>>,
    sessions_by_assignment: HashMap<AssignmentId, Vec<ReviewId>>,
    next_review: u64,
}

impl ReviewCore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request(
        &mut self,
        task_id: TaskId,
        target_assignment_id: AssignmentId,
        review_assignment_ids: Vec<AssignmentId>,
        criteria: ReviewCriteria,
        created_at: Timestamp,
    ) -> Result<ReviewId, ReviewError> {
        validate_review_assignments(&review_assignment_ids)?;
        validate_criteria(&criteria)?;

        let review_id = self.next_review_id();
        self.sessions.insert(
            review_id.clone(),
            ReviewSession {
                review_id: review_id.clone(),
                task_id: task_id.clone(),
                target_assignment_id: target_assignment_id.clone(),
                review_assignment_ids,
                criteria,
                verdicts: Vec::new(),
                created_at,
            },
        );
        self.sessions_by_task
            .entry(task_id)
            .or_default()
            .push(review_id.clone());
        self.sessions_by_assignment
            .entry(target_assignment_id)
            .or_default()
            .push(review_id.clone());

        Ok(review_id)
    }

    pub fn submit(
        &mut self,
        review_id: &ReviewId,
        artifact: ReviewArtifactEvidence,
        verdict: Verdict,
        submitted_at: Timestamp,
    ) -> Result<(), ReviewError> {
        validate_verdict(&verdict)?;
        validate_review_artifact(&artifact)?;
        let review_assignment_id = artifact.review_assignment_id.clone();

        let session = self
            .sessions
            .get_mut(review_id)
            .ok_or_else(|| ReviewError::ReviewNotFound(review_id.clone()))?;
        if !session
            .review_assignment_ids
            .contains(&review_assignment_id)
        {
            return Err(ReviewError::ReviewAssignmentNotAllowed {
                review_id: review_id.clone(),
                review_assignment_id,
            });
        }
        if session
            .verdicts
            .iter()
            .any(|record| record.review_assignment_id == review_assignment_id)
        {
            return Err(ReviewError::DuplicateVerdict {
                review_id: review_id.clone(),
                review_assignment_id,
            });
        }

        session.verdicts.push(VerdictRecord {
            review_id: review_id.clone(),
            review_assignment_id,
            artifact_hash: artifact.output_hash.expect("artifact was validated"),
            target_assignment_id: session.target_assignment_id.clone(),
            verdict,
            submitted_at,
        });
        Ok(())
    }

    pub fn collect(&self, review_id: &ReviewId) -> Option<Vec<VerdictRecord>> {
        self.sessions
            .get(review_id)
            .map(|session| session.verdicts.clone())
    }

    pub fn collect_by_assignment(&self, assignment_id: &AssignmentId) -> Vec<ReviewSession> {
        self.sessions_from_index(self.sessions_by_assignment.get(assignment_id))
    }

    pub fn collect_by_task(&self, task_id: &TaskId) -> Vec<ReviewSession> {
        self.sessions_from_index(self.sessions_by_task.get(task_id))
    }

    pub fn get(&self, review_id: &ReviewId) -> Option<&ReviewSession> {
        self.sessions.get(review_id)
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    fn sessions_from_index(&self, review_ids: Option<&Vec<ReviewId>>) -> Vec<ReviewSession> {
        let Some(review_ids) = review_ids else {
            return Vec::new();
        };
        review_ids
            .iter()
            .filter_map(|review_id| self.sessions.get(review_id).cloned())
            .collect()
    }

    fn next_review_id(&mut self) -> ReviewId {
        self.next_review += 1;
        ReviewId::new(format!("review-{}", self.next_review))
    }
}

fn validate_review_artifact(artifact: &ReviewArtifactEvidence) -> Result<(), ReviewError> {
    if artifact.status != AssignmentStatus::Submitted {
        return Err(ReviewError::ReviewArtifactNotSubmitted {
            review_assignment_id: artifact.review_assignment_id.clone(),
            status: artifact.status,
        });
    }
    if artifact.output_hash.is_none() {
        return Err(ReviewError::MissingReviewArtifactHash(
            artifact.review_assignment_id.clone(),
        ));
    }

    Ok(())
}

fn validate_review_assignments(review_assignment_ids: &[AssignmentId]) -> Result<(), ReviewError> {
    let mut seen = HashSet::new();
    for assignment_id in review_assignment_ids {
        if !seen.insert(assignment_id.clone()) {
            return Err(ReviewError::DuplicateReviewAssignment(
                assignment_id.clone(),
            ));
        }
    }

    Ok(())
}

fn validate_criteria(criteria: &ReviewCriteria) -> Result<(), ReviewError> {
    if criteria.body.trim().is_empty() {
        return Err(ReviewError::EmptyCriteria);
    }

    let actual_bytes = criteria.body.len();
    if actual_bytes > MAX_CRITERIA_BYTES {
        return Err(ReviewError::CriteriaTooLarge {
            max_bytes: MAX_CRITERIA_BYTES,
            actual_bytes,
        });
    }

    Ok(())
}

fn validate_verdict(verdict: &Verdict) -> Result<(), ReviewError> {
    if verdict.score_bps > MAX_SCORE_BPS {
        return Err(ReviewError::InvalidScore(verdict.score_bps));
    }

    let actual_bytes = verdict.feedback.len();
    if actual_bytes > MAX_FEEDBACK_BYTES {
        return Err(ReviewError::FeedbackTooLarge {
            max_bytes: MAX_FEEDBACK_BYTES,
            actual_bytes,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::review::{CriteriaFormat, VerdictKind};
    use crate::types::OutputHash;

    use super::*;

    fn task(id: &str) -> TaskId {
        TaskId::from(id)
    }

    fn assignment(id: &str) -> AssignmentId {
        AssignmentId::from(id)
    }

    fn criteria() -> ReviewCriteria {
        ReviewCriteria::plain_text("check correctness")
    }

    fn verdict(kind: VerdictKind, score_bps: u16) -> Verdict {
        Verdict {
            kind,
            score_bps,
            feedback: "looks good".to_string(),
        }
    }

    fn submitted_artifact(id: &str) -> ReviewArtifactEvidence {
        ReviewArtifactEvidence::new(
            assignment(id),
            AssignmentStatus::Submitted,
            Some(OutputHash::from(format!("artifact-hash-{id}"))),
        )
    }

    fn request_review(core: &mut ReviewCore) -> ReviewId {
        core.request(
            task("task-1"),
            assignment("execute-1"),
            vec![assignment("review-1"), assignment("review-2")],
            criteria(),
            Timestamp(1),
        )
        .unwrap()
    }

    #[test]
    fn request_creates_session_and_indexes() {
        let mut core = ReviewCore::new();

        let review_id = request_review(&mut core);
        let session = core.get(&review_id).unwrap();

        assert_eq!(core.session_count(), 1);
        assert_eq!(session.review_id, review_id);
        assert_eq!(session.task_id, task("task-1"));
        assert_eq!(session.target_assignment_id, assignment("execute-1"));
        assert_eq!(session.review_assignment_ids.len(), 2);
        assert_eq!(session.criteria.format, CriteriaFormat::PlainText);
        assert_eq!(core.collect_by_task(&task("task-1")).len(), 1);
        assert_eq!(
            core.collect_by_assignment(&assignment("execute-1")).len(),
            1
        );
    }

    #[test]
    fn request_allows_empty_review_assignments_but_rejects_duplicates() {
        let mut core = ReviewCore::new();

        let review_id = core
            .request(
                task("task-1"),
                assignment("execute-1"),
                Vec::new(),
                criteria(),
                Timestamp(1),
            )
            .unwrap();
        assert!(
            core.get(&review_id)
                .unwrap()
                .review_assignment_ids
                .is_empty()
        );

        assert_eq!(
            core.request(
                task("task-2"),
                assignment("execute-2"),
                vec![assignment("review-1"), assignment("review-1")],
                criteria(),
                Timestamp(1),
            )
            .unwrap_err(),
            ReviewError::DuplicateReviewAssignment(assignment("review-1"))
        );
    }

    #[test]
    fn request_rejects_empty_or_large_criteria() {
        let mut core = ReviewCore::new();

        assert_eq!(
            core.request(
                task("task-1"),
                assignment("execute-1"),
                vec![assignment("review-1")],
                ReviewCriteria::plain_text(" "),
                Timestamp(1),
            )
            .unwrap_err(),
            ReviewError::EmptyCriteria
        );

        let body = "x".repeat(MAX_CRITERIA_BYTES + 1);
        assert_eq!(
            core.request(
                task("task-1"),
                assignment("execute-1"),
                vec![assignment("review-1")],
                ReviewCriteria::plain_text(body),
                Timestamp(1),
            )
            .unwrap_err(),
            ReviewError::CriteriaTooLarge {
                max_bytes: MAX_CRITERIA_BYTES,
                actual_bytes: MAX_CRITERIA_BYTES + 1
            }
        );
    }

    #[test]
    fn submit_records_authorized_verdict() {
        let mut core = ReviewCore::new();
        let review_id = request_review(&mut core);

        core.submit(
            &review_id,
            submitted_artifact("review-1"),
            verdict(VerdictKind::Passed, 9_000),
            Timestamp(2),
        )
        .unwrap();

        let verdicts = core.collect(&review_id).unwrap();
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].review_id, review_id);
        assert_eq!(verdicts[0].review_assignment_id, assignment("review-1"));
        assert_eq!(verdicts[0].target_assignment_id, assignment("execute-1"));
    }

    #[test]
    fn submit_rejects_unknown_or_unauthorized_review_assignment() {
        let mut core = ReviewCore::new();
        let review_id = request_review(&mut core);

        assert_eq!(
            core.submit(
                &ReviewId::from("missing"),
                submitted_artifact("review-1"),
                verdict(VerdictKind::Passed, 9_000),
                Timestamp(2),
            )
            .unwrap_err(),
            ReviewError::ReviewNotFound(ReviewId::from("missing"))
        );

        assert_eq!(
            core.submit(
                &review_id,
                submitted_artifact("review-3"),
                verdict(VerdictKind::Passed, 9_000),
                Timestamp(2),
            )
            .unwrap_err(),
            ReviewError::ReviewAssignmentNotAllowed {
                review_id,
                review_assignment_id: assignment("review-3")
            }
        );
    }

    #[test]
    fn submit_rejects_duplicate_verdict() {
        let mut core = ReviewCore::new();
        let review_id = request_review(&mut core);

        core.submit(
            &review_id,
            submitted_artifact("review-1"),
            verdict(VerdictKind::Passed, 9_000),
            Timestamp(2),
        )
        .unwrap();

        assert_eq!(
            core.submit(
                &review_id,
                submitted_artifact("review-1"),
                verdict(VerdictKind::Failed, 1_000),
                Timestamp(3),
            )
            .unwrap_err(),
            ReviewError::DuplicateVerdict {
                review_id,
                review_assignment_id: assignment("review-1")
            }
        );
    }

    #[test]
    fn submit_rejects_invalid_score_and_large_feedback() {
        let mut core = ReviewCore::new();
        let review_id = request_review(&mut core);

        assert_eq!(
            core.submit(
                &review_id,
                submitted_artifact("review-1"),
                verdict(VerdictKind::Passed, 10_001),
                Timestamp(2),
            )
            .unwrap_err(),
            ReviewError::InvalidScore(10_001)
        );

        let large = Verdict {
            kind: VerdictKind::Passed,
            score_bps: 9_000,
            feedback: "x".repeat(MAX_FEEDBACK_BYTES + 1),
        };
        assert_eq!(
            core.submit(
                &review_id,
                submitted_artifact("review-1"),
                large,
                Timestamp(2)
            )
            .unwrap_err(),
            ReviewError::FeedbackTooLarge {
                max_bytes: MAX_FEEDBACK_BYTES,
                actual_bytes: MAX_FEEDBACK_BYTES + 1
            }
        );
    }

    #[test]
    fn submit_requires_submitted_review_artifact() {
        let mut core = ReviewCore::new();
        let review_id = request_review(&mut core);

        assert_eq!(
            core.submit(
                &review_id,
                ReviewArtifactEvidence::new(
                    assignment("review-1"),
                    AssignmentStatus::Assigned,
                    None
                ),
                verdict(VerdictKind::Passed, 9_000),
                Timestamp(2),
            )
            .unwrap_err(),
            ReviewError::ReviewArtifactNotSubmitted {
                review_assignment_id: assignment("review-1"),
                status: AssignmentStatus::Assigned
            }
        );
        assert_eq!(
            core.submit(
                &review_id,
                ReviewArtifactEvidence::new(
                    assignment("review-1"),
                    AssignmentStatus::Submitted,
                    None
                ),
                verdict(VerdictKind::Passed, 9_000),
                Timestamp(2),
            )
            .unwrap_err(),
            ReviewError::MissingReviewArtifactHash(assignment("review-1"))
        );
    }
}
