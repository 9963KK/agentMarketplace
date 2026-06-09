use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::heartbeat::AgentId;
use crate::review::ReviewId;
use crate::types::{AssignmentId, TaskId, Timestamp};

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct HoldId(String);

impl HoldId {
    pub fn new(value: impl Into<String>) -> Self {
        Self::try_new(value).expect("hold id must not be empty")
    }

    pub fn try_new(value: impl Into<String>) -> Result<Self, SettlementError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(SettlementError::EmptyHoldId);
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for HoldId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for HoldId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for HoldId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

pub type Balance = u64;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Hold {
    pub hold_id: HoldId,
    pub from_agent: AgentId,
    pub amount: u64,
    pub task_id: TaskId,
    pub assignment_id: AssignmentId,
    pub agent_id: AgentId,
    pub kind: HoldKind,
    pub status: HoldStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HoldRequest {
    pub from_agent: AgentId,
    pub amount: u64,
    pub task_id: TaskId,
    pub assignment_id: AssignmentId,
    pub agent_id: AgentId,
    pub kind: HoldKind,
}

impl HoldRequest {
    pub fn new(
        from_agent: impl Into<AgentId>,
        amount: u64,
        task_id: impl Into<TaskId>,
        assignment_id: impl Into<AssignmentId>,
        agent_id: impl Into<AgentId>,
        kind: HoldKind,
    ) -> Self {
        Self {
            from_agent: from_agent.into(),
            amount,
            task_id: task_id.into(),
            assignment_id: assignment_id.into(),
            agent_id: agent_id.into(),
            kind,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum HoldKind {
    Execute,
    Review,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum HoldStatus {
    Active,
    Released,
    Refunded,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ReleaseEvidence {
    AssignmentOutputAccepted {
        task_id: TaskId,
        assignment_id: AssignmentId,
        review_ids: Vec<ReviewId>,
    },
    ReviewSubmitted {
        task_id: TaskId,
        assignment_id: AssignmentId,
        review_id: ReviewId,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LedgerEntry {
    pub hold_id: Option<HoldId>,
    pub task_id: Option<TaskId>,
    pub assignment_id: Option<AssignmentId>,
    pub amount: u64,
    pub kind: LedgerEntryKind,
    pub at: Timestamp,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum LedgerEntryKind {
    Deposited {
        agent_id: AgentId,
    },
    HoldCreated {
        from_agent: AgentId,
        agent_id: AgentId,
    },
    Released {
        to_agent: AgentId,
    },
    Refunded {
        to_agent: AgentId,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SettlementOutcome {
    Held(HoldId),
    Released,
    Refunded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettlementError {
    EmptyHoldId,
    ZeroAmount,
    EmptyReviewEvidence,
    InsufficientBalance {
        agent_id: AgentId,
        available: Balance,
        required: Balance,
    },
    HoldNotFound(HoldId),
    HoldNotActive {
        hold_id: HoldId,
        status: HoldStatus,
    },
    ReleaseEvidenceMismatch {
        hold_id: HoldId,
    },
    HoldKindMismatch {
        hold_id: HoldId,
        expected: HoldKind,
        actual: HoldKind,
    },
    Overflow,
}

impl fmt::Display for SettlementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettlementError::EmptyHoldId => f.write_str("hold id must not be empty"),
            SettlementError::ZeroAmount => f.write_str("amount must be greater than zero"),
            SettlementError::EmptyReviewEvidence => {
                f.write_str("release evidence must include at least one review")
            }
            SettlementError::InsufficientBalance {
                agent_id,
                available,
                required,
            } => write!(
                f,
                "insufficient balance for {agent_id}: available={available}, required={required}"
            ),
            SettlementError::HoldNotFound(hold_id) => write!(f, "hold not found: {hold_id}"),
            SettlementError::HoldNotActive { hold_id, status } => {
                write!(f, "hold is not active: {hold_id}, status={status:?}")
            }
            SettlementError::ReleaseEvidenceMismatch { hold_id } => {
                write!(f, "release evidence does not match hold: {hold_id}")
            }
            SettlementError::HoldKindMismatch {
                hold_id,
                expected,
                actual,
            } => write!(
                f,
                "release evidence kind mismatch for hold {hold_id}: expected={expected:?}, actual={actual:?}"
            ),
            SettlementError::Overflow => f.write_str("balance overflow"),
        }
    }
}

impl Error for SettlementError {}
