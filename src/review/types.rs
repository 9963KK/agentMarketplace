use std::error::Error;
use std::fmt;

use crate::types::{AssignmentId, TaskId, Timestamp};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ReviewId(String);

impl ReviewId {
    pub fn new(value: impl Into<String>) -> Self {
        Self::try_new(value).expect("review id must not be empty")
    }

    pub fn try_new(value: impl Into<String>) -> Result<Self, ReviewError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ReviewError::EmptyReviewId);
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ReviewId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ReviewId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for ReviewId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CriteriaFormat {
    PlainText,
    Json,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewCriteria {
    pub format: CriteriaFormat,
    pub body: String,
}

impl ReviewCriteria {
    pub fn plain_text(body: impl Into<String>) -> Self {
        Self {
            format: CriteriaFormat::PlainText,
            body: body.into(),
        }
    }

    pub fn json(body: impl Into<String>) -> Self {
        Self {
            format: CriteriaFormat::Json,
            body: body.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewSession {
    pub review_id: ReviewId,
    pub task_id: TaskId,
    pub target_assignment_id: AssignmentId,
    pub review_assignment_ids: Vec<AssignmentId>,
    pub criteria: ReviewCriteria,
    pub verdicts: Vec<VerdictRecord>,
    pub created_at: Timestamp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerdictRecord {
    pub review_id: ReviewId,
    pub review_assignment_id: AssignmentId,
    pub target_assignment_id: AssignmentId,
    pub verdict: Verdict,
    pub submitted_at: Timestamp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Verdict {
    pub kind: VerdictKind,
    pub score_bps: u16,
    pub feedback: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VerdictKind {
    Passed,
    Failed,
    ArtifactUnavailable,
    HashMismatch,
    InvalidFormat,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewError {
    EmptyReviewId,
    EmptyCriteria,
    CriteriaTooLarge {
        max_bytes: usize,
        actual_bytes: usize,
    },
    FeedbackTooLarge {
        max_bytes: usize,
        actual_bytes: usize,
    },
    InvalidScore(u16),
    DuplicateReviewAssignment(AssignmentId),
    ReviewNotFound(ReviewId),
    ReviewAssignmentNotAllowed {
        review_id: ReviewId,
        review_assignment_id: AssignmentId,
    },
    DuplicateVerdict {
        review_id: ReviewId,
        review_assignment_id: AssignmentId,
    },
}

impl fmt::Display for ReviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReviewError::EmptyReviewId => f.write_str("review id must not be empty"),
            ReviewError::EmptyCriteria => f.write_str("review criteria must not be empty"),
            ReviewError::CriteriaTooLarge {
                max_bytes,
                actual_bytes,
            } => write!(
                f,
                "review criteria too large: max={max_bytes}, actual={actual_bytes}"
            ),
            ReviewError::FeedbackTooLarge {
                max_bytes,
                actual_bytes,
            } => write!(
                f,
                "review feedback too large: max={max_bytes}, actual={actual_bytes}"
            ),
            ReviewError::InvalidScore(score) => {
                write!(f, "review score must be <= 10000: {score}")
            }
            ReviewError::DuplicateReviewAssignment(assignment_id) => {
                write!(f, "duplicate review assignment: {assignment_id}")
            }
            ReviewError::ReviewNotFound(review_id) => write!(f, "review not found: {review_id}"),
            ReviewError::ReviewAssignmentNotAllowed {
                review_id,
                review_assignment_id,
            } => write!(
                f,
                "review assignment {review_assignment_id} is not allowed for review {review_id}"
            ),
            ReviewError::DuplicateVerdict {
                review_id,
                review_assignment_id,
            } => write!(
                f,
                "duplicate verdict for review {review_id} by review assignment {review_assignment_id}"
            ),
        }
    }
}

impl Error for ReviewError {}
