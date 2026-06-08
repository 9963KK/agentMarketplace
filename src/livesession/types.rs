use std::collections::HashSet;
use std::error::Error;
use std::fmt;

use crate::heartbeat::AgentId;
use crate::types::{AssignmentId, OutputHash, SessionId, TaskId, Timestamp};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveSession {
    pub session_id: SessionId,
    pub task_id: TaskId,
    pub assignment_ids: HashSet<AssignmentId>,
    pub status: LiveSessionStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveSessionStatus {
    Running,
    Closed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Assignment {
    pub assignment_id: AssignmentId,
    pub task_id: TaskId,
    pub session_id: SessionId,
    pub agent_id: AgentId,
    pub kind: AssignmentKind,
    pub status: AssignmentStatus,
    pub output_hash: Option<OutputHash>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssignmentKind {
    Execute,
    Review { target_assignment_id: AssignmentId },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssignmentStatus {
    Assigned,
    Submitted,
    Approved,
    Rejected,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LiveSessionError {
    SessionNotFound(SessionId),
    SessionNotRunning {
        session_id: SessionId,
        status: LiveSessionStatus,
    },
    AssignmentNotFound(AssignmentId),
    AssignmentNotAssigned {
        assignment_id: AssignmentId,
        status: AssignmentStatus,
    },
    AssignmentNotSubmitted {
        assignment_id: AssignmentId,
        status: AssignmentStatus,
    },
    AgentMismatch {
        assignment_id: AssignmentId,
        expected: AgentId,
        actual: AgentId,
    },
    TargetAssignmentNotFound(AssignmentId),
    TargetAssignmentTaskMismatch {
        target_assignment_id: AssignmentId,
        expected_task_id: TaskId,
        actual_task_id: TaskId,
    },
    TargetAssignmentKindMismatch {
        target_assignment_id: AssignmentId,
        kind: AssignmentKind,
    },
    SessionTaskMismatch {
        session_id: SessionId,
        expected_task_id: TaskId,
        actual_task_id: TaskId,
    },
    TimestampWentBackwards {
        current: Timestamp,
        attempted: Timestamp,
    },
}

impl fmt::Display for LiveSessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LiveSessionError::SessionNotFound(session_id) => {
                write!(f, "live session not found: {session_id}")
            }
            LiveSessionError::SessionNotRunning { session_id, status } => {
                write!(
                    f,
                    "live session is not running: {session_id}, status={status:?}"
                )
            }
            LiveSessionError::AssignmentNotFound(assignment_id) => {
                write!(f, "assignment not found: {assignment_id}")
            }
            LiveSessionError::AssignmentNotAssigned {
                assignment_id,
                status,
            } => write!(
                f,
                "assignment is not assigned: {assignment_id}, status={status:?}"
            ),
            LiveSessionError::AssignmentNotSubmitted {
                assignment_id,
                status,
            } => write!(
                f,
                "assignment is not submitted: {assignment_id}, status={status:?}"
            ),
            LiveSessionError::AgentMismatch {
                assignment_id,
                expected,
                actual,
            } => write!(
                f,
                "agent mismatch for assignment {assignment_id}: expected={expected}, actual={actual}"
            ),
            LiveSessionError::TargetAssignmentNotFound(assignment_id) => {
                write!(f, "target assignment not found: {assignment_id}")
            }
            LiveSessionError::TargetAssignmentTaskMismatch {
                target_assignment_id,
                expected_task_id,
                actual_task_id,
            } => write!(
                f,
                "target assignment task mismatch: {target_assignment_id}, expected={expected_task_id}, actual={actual_task_id}"
            ),
            LiveSessionError::TargetAssignmentKindMismatch {
                target_assignment_id,
                kind,
            } => write!(
                f,
                "target assignment must be executable: {target_assignment_id}, kind={kind:?}"
            ),
            LiveSessionError::SessionTaskMismatch {
                session_id,
                expected_task_id,
                actual_task_id,
            } => write!(
                f,
                "live session task mismatch: {session_id}, expected={expected_task_id}, actual={actual_task_id}"
            ),
            LiveSessionError::TimestampWentBackwards { current, attempted } => write!(
                f,
                "live session timestamp went backwards: current={}, attempted={}",
                current.0, attempted.0
            ),
        }
    }
}

impl Error for LiveSessionError {}
