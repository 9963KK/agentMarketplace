use std::collections::{HashMap, HashSet};

use crate::heartbeat::AgentId;
use crate::types::{OutputHash, TaskId, Timestamp};

use super::types::{ReviewCriteria, ReviewError, ReviewId, ReviewSession, Verdict, VerdictRecord};

pub const MAX_CRITERIA_BYTES: usize = 16 * 1024;
pub const MAX_FEEDBACK_BYTES: usize = 32 * 1024;
const MAX_SCORE_BPS: u16 = 10_000;

#[derive(Debug, Default)]
pub struct ReviewCore {
    sessions: HashMap<ReviewId, ReviewSession>,
    sessions_by_task: HashMap<TaskId, Vec<ReviewId>>,
    next_review: u64,
}

impl ReviewCore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request(
        &mut self,
        task_id: TaskId,
        executor_id: AgentId,
        output_hash: OutputHash,
        allowed_reviewers: Vec<AgentId>,
        criteria: ReviewCriteria,
        created_at: Timestamp,
    ) -> Result<ReviewId, ReviewError> {
        validate_reviewers(&allowed_reviewers)?;
        validate_criteria(&criteria)?;

        let review_id = self.next_review_id();
        self.sessions.insert(
            review_id.clone(),
            ReviewSession {
                review_id: review_id.clone(),
                task_id: task_id.clone(),
                executor_id,
                output_hash,
                allowed_reviewers,
                criteria,
                verdicts: Vec::new(),
                created_at,
            },
        );
        self.sessions_by_task
            .entry(task_id)
            .or_default()
            .push(review_id.clone());

        Ok(review_id)
    }

    pub fn submit(
        &mut self,
        review_id: &ReviewId,
        reviewer_id: AgentId,
        verdict: Verdict,
        submitted_at: Timestamp,
    ) -> Result<(), ReviewError> {
        validate_verdict(&verdict)?;

        let session = self
            .sessions
            .get_mut(review_id)
            .ok_or_else(|| ReviewError::ReviewNotFound(review_id.clone()))?;
        if !session.allowed_reviewers.contains(&reviewer_id) {
            return Err(ReviewError::ReviewerNotAllowed {
                review_id: review_id.clone(),
                reviewer_id,
            });
        }
        if session
            .verdicts
            .iter()
            .any(|record| record.reviewer_id == reviewer_id)
        {
            return Err(ReviewError::DuplicateVerdict {
                review_id: review_id.clone(),
                reviewer_id,
            });
        }

        session.verdicts.push(VerdictRecord {
            review_id: review_id.clone(),
            reviewer_id,
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

    pub fn collect_by_task(&self, task_id: &TaskId) -> Vec<ReviewSession> {
        let Some(review_ids) = self.sessions_by_task.get(task_id) else {
            return Vec::new();
        };

        review_ids
            .iter()
            .filter_map(|review_id| self.sessions.get(review_id).cloned())
            .collect()
    }

    pub fn get(&self, review_id: &ReviewId) -> Option<&ReviewSession> {
        self.sessions.get(review_id)
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    fn next_review_id(&mut self) -> ReviewId {
        self.next_review += 1;
        ReviewId::new(format!("review-{}", self.next_review))
    }
}

fn validate_reviewers(reviewers: &[AgentId]) -> Result<(), ReviewError> {
    let mut seen = HashSet::new();
    for reviewer in reviewers {
        if !seen.insert(reviewer.clone()) {
            return Err(ReviewError::DuplicateReviewer(reviewer.clone()));
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

    fn agent(id: &str) -> AgentId {
        AgentId::from(id)
    }

    fn task(id: &str) -> TaskId {
        TaskId::from(id)
    }

    fn output_hash(value: &str) -> OutputHash {
        OutputHash::from(value)
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

    fn request_review(core: &mut ReviewCore) -> ReviewId {
        core.request(
            task("task-1"),
            agent("executor-1"),
            output_hash("hash-1"),
            vec![agent("reviewer-1"), agent("reviewer-2")],
            criteria(),
            Timestamp(1),
        )
        .unwrap()
    }

    #[test]
    fn request_creates_session_and_task_index() {
        let mut core = ReviewCore::new();

        let review_id = request_review(&mut core);
        let session = core.get(&review_id).unwrap();

        assert_eq!(core.session_count(), 1);
        assert_eq!(session.review_id, review_id);
        assert_eq!(session.task_id, task("task-1"));
        assert_eq!(session.executor_id, agent("executor-1"));
        assert_eq!(session.output_hash, output_hash("hash-1"));
        assert_eq!(session.allowed_reviewers.len(), 2);
        assert_eq!(session.criteria.format, CriteriaFormat::PlainText);
        assert_eq!(core.collect_by_task(&task("task-1")).len(), 1);
    }

    #[test]
    fn request_allows_empty_reviewers_but_rejects_duplicate_reviewers() {
        let mut core = ReviewCore::new();

        let review_id = core
            .request(
                task("task-1"),
                agent("executor-1"),
                output_hash("hash-1"),
                Vec::new(),
                criteria(),
                Timestamp(1),
            )
            .unwrap();
        assert!(core.get(&review_id).unwrap().allowed_reviewers.is_empty());

        assert_eq!(
            core.request(
                task("task-2"),
                agent("executor-1"),
                output_hash("hash-2"),
                vec![agent("reviewer-1"), agent("reviewer-1")],
                criteria(),
                Timestamp(1),
            )
            .unwrap_err(),
            ReviewError::DuplicateReviewer(agent("reviewer-1"))
        );
    }

    #[test]
    fn request_rejects_empty_or_large_criteria() {
        let mut core = ReviewCore::new();

        assert_eq!(
            core.request(
                task("task-1"),
                agent("executor-1"),
                output_hash("hash-1"),
                vec![agent("reviewer-1")],
                ReviewCriteria::plain_text(" "),
                Timestamp(1),
            )
            .unwrap_err(),
            ReviewError::EmptyCriteria
        );

        let large = "x".repeat(MAX_CRITERIA_BYTES + 1);
        assert_eq!(
            core.request(
                task("task-1"),
                agent("executor-1"),
                output_hash("hash-1"),
                vec![agent("reviewer-1")],
                ReviewCriteria::json(large),
                Timestamp(1),
            )
            .unwrap_err(),
            ReviewError::CriteriaTooLarge {
                max_bytes: MAX_CRITERIA_BYTES,
                actual_bytes: MAX_CRITERIA_BYTES + 1,
            }
        );
    }

    #[test]
    fn submit_records_authorized_verdict() {
        let mut core = ReviewCore::new();
        let review_id = request_review(&mut core);

        core.submit(
            &review_id,
            agent("reviewer-1"),
            verdict(VerdictKind::Passed, 9_500),
            Timestamp(2),
        )
        .unwrap();

        let verdicts = core.collect(&review_id).unwrap();
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].reviewer_id, agent("reviewer-1"));
        assert_eq!(verdicts[0].submitted_at, Timestamp(2));
    }

    #[test]
    fn submit_rejects_unknown_or_unauthorized_review() {
        let mut core = ReviewCore::new();
        let review_id = request_review(&mut core);

        assert_eq!(
            core.submit(
                &ReviewId::from("missing"),
                agent("reviewer-1"),
                verdict(VerdictKind::Passed, 10_000),
                Timestamp(2),
            )
            .unwrap_err(),
            ReviewError::ReviewNotFound(ReviewId::from("missing"))
        );
        assert_eq!(
            core.submit(
                &review_id,
                agent("intruder"),
                verdict(VerdictKind::Passed, 10_000),
                Timestamp(2),
            )
            .unwrap_err(),
            ReviewError::ReviewerNotAllowed {
                review_id,
                reviewer_id: agent("intruder"),
            }
        );
    }

    #[test]
    fn submit_rejects_duplicate_verdict() {
        let mut core = ReviewCore::new();
        let review_id = request_review(&mut core);

        core.submit(
            &review_id,
            agent("reviewer-1"),
            verdict(VerdictKind::Passed, 10_000),
            Timestamp(2),
        )
        .unwrap();

        assert_eq!(
            core.submit(
                &review_id,
                agent("reviewer-1"),
                verdict(VerdictKind::Failed, 0),
                Timestamp(3),
            )
            .unwrap_err(),
            ReviewError::DuplicateVerdict {
                review_id,
                reviewer_id: agent("reviewer-1"),
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
                agent("reviewer-1"),
                verdict(VerdictKind::Passed, 10_001),
                Timestamp(2),
            )
            .unwrap_err(),
            ReviewError::InvalidScore(10_001)
        );

        let mut large_feedback = verdict(VerdictKind::Failed, 0);
        large_feedback.feedback = "x".repeat(MAX_FEEDBACK_BYTES + 1);
        assert_eq!(
            core.submit(
                &review_id,
                agent("reviewer-1"),
                large_feedback,
                Timestamp(2),
            )
            .unwrap_err(),
            ReviewError::FeedbackTooLarge {
                max_bytes: MAX_FEEDBACK_BYTES,
                actual_bytes: MAX_FEEDBACK_BYTES + 1,
            }
        );
    }
}
