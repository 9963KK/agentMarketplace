use std::collections::HashSet;
use std::error::Error;
use std::fmt;

use crate::heartbeat::AgentId;
use crate::types::{TaskId, Timestamp};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Task {
    pub task_id: TaskId,
    pub publisher: AgentId,
    pub active_participants: HashSet<AgentId>,
    pub participant_history: HashSet<AgentId>,
    pub status: TaskStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskStatus {
    Active,
    Completed,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TaskError {
    TaskNotFound(TaskId),
    TaskNotActive {
        task_id: TaskId,
        status: TaskStatus,
    },
    TimestampWentBackwards {
        task_id: TaskId,
        current: Timestamp,
        attempted: Timestamp,
    },
}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskError::TaskNotFound(task_id) => write!(f, "task not found: {task_id}"),
            TaskError::TaskNotActive { task_id, status } => {
                write!(f, "task is not active: {task_id}, status={status:?}")
            }
            TaskError::TimestampWentBackwards {
                task_id,
                current,
                attempted,
            } => write!(
                f,
                "task timestamp went backwards: {task_id}, current={}, attempted={}",
                current.0, attempted.0
            ),
        }
    }
}

impl Error for TaskError {}
