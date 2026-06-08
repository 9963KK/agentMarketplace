use crate::heartbeat::{AgentId, HeartbeatEvent};
use crate::settlement::HoldId;
use crate::types::{AssignmentId, TaskId, Timestamp};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeEventReport {
    pub event: HeartbeatEvent,
    pub at: Timestamp,
    pub actions: Vec<RuntimeAction>,
    pub errors: Vec<RuntimeActionError>,
}

impl RuntimeEventReport {
    pub fn new(event: HeartbeatEvent, at: Timestamp) -> Self {
        Self {
            event,
            at,
            actions: Vec::new(),
            errors: Vec::new(),
        }
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub(crate) fn record_action(&mut self, action: RuntimeAction) {
        self.actions.push(action);
    }

    pub(crate) fn record_error(
        &mut self,
        kind: RuntimeActionKind,
        target: impl Into<String>,
        error: impl ToString,
    ) {
        self.errors.push(RuntimeActionError {
            kind,
            target: target.into(),
            message: error.to_string(),
        });
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeAction {
    RegistryMarkedTimedOut { agent_id: AgentId },
    RegistryMarkedAlive { agent_id: AgentId },
    HoldRefunded { hold_id: HoldId },
    AssignmentCancelled { assignment_id: AssignmentId },
    TaskParticipantRemoved { task_id: TaskId, agent_id: AgentId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeActionError {
    pub kind: RuntimeActionKind,
    pub target: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeActionKind {
    MarkRegistryTimedOut,
    MarkRegistryAlive,
    ListActiveHoldsForAgent,
    RefundHold,
    ListAssignmentsByAgent,
    CancelAssignment,
    ListActiveTasksByAgent,
    RemoveTaskParticipant,
}
