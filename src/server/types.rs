use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::heartbeat::{AgentId, HeartbeatServiceStartError, PingOutcome};
use crate::livesession::{Assignment, AssignmentKind};
use crate::registry::RegisterOutcome;
use crate::review::{ReviewCriteria, ReviewId};
use crate::settlement::HoldId;
use crate::storage::ArtifactLocator;
use crate::types::{AssignmentId, OutputHash, SessionId, TaskId};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AgentToken(String);

impl AgentToken {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        assert!(!value.trim().is_empty(), "agent token must not be empty");
        Self(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for AgentToken {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for AgentToken {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for AgentToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RegisterAgentResponse {
    pub agent_id: AgentId,
    pub outcome: RegisterOutcome,
    pub token: AgentToken,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PingResponse {
    pub agent_id: AgentId,
    pub outcome: PingOutcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SubmittedArtifact {
    pub assignment_id: AssignmentId,
    pub manifest_hash: OutputHash,
    pub locator: ArtifactLocator,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreatedSession {
    pub session_id: SessionId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreatedAssignment {
    pub assignment_id: AssignmentId,
    pub kind: AssignmentKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AssignRequest {
    pub task_id: TaskId,
    pub session_id: SessionId,
    pub agent_id: AgentId,
    pub kind: AssignmentKind,
}

impl AssignRequest {
    pub fn new(
        task_id: impl Into<TaskId>,
        session_id: impl Into<SessionId>,
        agent_id: impl Into<AgentId>,
        kind: AssignmentKind,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            session_id: session_id.into(),
            agent_id: agent_id.into(),
            kind,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestedReview {
    pub review_id: ReviewId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReviewRequest {
    pub task_id: TaskId,
    pub target_assignment_id: AssignmentId,
    pub review_assignment_ids: Vec<AssignmentId>,
    pub criteria: ReviewCriteria,
}

impl ReviewRequest {
    pub fn new(
        task_id: impl Into<TaskId>,
        target_assignment_id: impl Into<AssignmentId>,
        review_assignment_ids: Vec<AssignmentId>,
        criteria: ReviewCriteria,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            target_assignment_id: target_assignment_id.into(),
            review_assignment_ids,
            criteria,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreatedHold {
    pub hold_id: HoldId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServerError {
    Startup(String),
    Unauthorized,
    Forbidden {
        agent_id: AgentId,
        action: &'static str,
    },
    NotFound(String),
    BadRequest(String),
    IdempotencyInProgress,
    InvalidReplay {
        expected: &'static str,
        actual: String,
    },
    MissingAssignmentOutput(AssignmentId),
    InvalidAssignmentKind {
        assignment_id: AssignmentId,
        expected: &'static str,
        actual: AssignmentKind,
    },
    Component {
        component: &'static str,
        message: String,
    },
}

impl ServerError {
    pub(crate) fn component(component: &'static str, error: impl ToString) -> Self {
        Self::Component {
            component,
            message: error.to_string(),
        }
    }
}

impl From<HeartbeatServiceStartError> for ServerError {
    fn from(error: HeartbeatServiceStartError) -> Self {
        ServerError::Startup(error.to_string())
    }
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServerError::Startup(message) => write!(f, "server startup failed: {message}"),
            ServerError::Unauthorized => f.write_str("request is unauthorized"),
            ServerError::Forbidden { agent_id, action } => {
                write!(f, "agent {agent_id} is not allowed to {action}")
            }
            ServerError::NotFound(message) => write!(f, "not found: {message}"),
            ServerError::BadRequest(message) => write!(f, "bad request: {message}"),
            ServerError::IdempotencyInProgress => {
                f.write_str("idempotent operation is still in progress")
            }
            ServerError::InvalidReplay { expected, actual } => {
                write!(
                    f,
                    "invalid idempotent replay: expected {expected}, actual {actual}"
                )
            }
            ServerError::MissingAssignmentOutput(assignment_id) => {
                write!(f, "assignment is missing output hash: {assignment_id}")
            }
            ServerError::InvalidAssignmentKind {
                assignment_id,
                expected,
                actual,
            } => write!(
                f,
                "invalid assignment kind for {assignment_id}: expected={expected}, actual={actual:?}"
            ),
            ServerError::Component { component, message } => {
                write!(f, "{component} error: {message}")
            }
        }
    }
}

impl Error for ServerError {}

pub(crate) fn require_assignment_agent(
    assignment: &Assignment,
    agent_id: &AgentId,
    action: &'static str,
) -> Result<(), ServerError> {
    if assignment.agent_id != *agent_id {
        return Err(ServerError::Forbidden {
            agent_id: agent_id.clone(),
            action,
        });
    }

    Ok(())
}

pub(crate) fn require_task_publisher(
    publisher: &AgentId,
    agent_id: &AgentId,
    action: &'static str,
) -> Result<(), ServerError> {
    if publisher != agent_id {
        return Err(ServerError::Forbidden {
            agent_id: agent_id.clone(),
            action,
        });
    }

    Ok(())
}

pub(crate) fn parse_task_id(value: String) -> Result<TaskId, ServerError> {
    if let Some(task_id) = value.strip_prefix("task:") {
        return Ok(TaskId::from(task_id.to_string()));
    }

    Err(ServerError::InvalidReplay {
        expected: "task:<task_id>",
        actual: value,
    })
}

pub(crate) fn parse_session_id(value: String) -> Result<SessionId, ServerError> {
    if let Some(session_id) = value.strip_prefix("session:") {
        return Ok(SessionId::from(session_id.to_string()));
    }

    Err(ServerError::InvalidReplay {
        expected: "session:<session_id>",
        actual: value,
    })
}

pub(crate) fn parse_assignment_id(value: String) -> Result<AssignmentId, ServerError> {
    if let Some(assignment_id) = value.strip_prefix("assignment:") {
        return Ok(AssignmentId::from(assignment_id.to_string()));
    }

    Err(ServerError::InvalidReplay {
        expected: "assignment:<assignment_id>",
        actual: value,
    })
}

pub(crate) fn parse_review_id(value: String) -> Result<ReviewId, ServerError> {
    if let Some(review_id) = value.strip_prefix("review:") {
        return Ok(ReviewId::from(review_id.to_string()));
    }

    Err(ServerError::InvalidReplay {
        expected: "review:<review_id>",
        actual: value,
    })
}

pub(crate) fn parse_hold_id(value: String) -> Result<HoldId, ServerError> {
    if let Some(hold_id) = value.strip_prefix("hold:") {
        return Ok(HoldId::from(hold_id.to_string()));
    }

    Err(ServerError::InvalidReplay {
        expected: "hold:<hold_id>",
        actual: value,
    })
}
