use std::error::Error;
use std::fmt;

use tokio::sync::{mpsc, oneshot};

use crate::heartbeat::AgentId;
use crate::types::{AssignmentId, OutputHash, SessionId, TaskId, Timestamp};

use super::LiveSessionCore;
use super::types::{Assignment, AssignmentKind, LiveSession, LiveSessionError};

const DEFAULT_COMMAND_BUFFER: usize = 128;

#[derive(Debug)]
pub enum LiveSessionCommand {
    CreateSession {
        task_id: TaskId,
        at: Timestamp,
        reply: oneshot::Sender<SessionId>,
    },
    CloseSession {
        session_id: SessionId,
        at: Timestamp,
        reply: oneshot::Sender<Result<(), LiveSessionError>>,
    },
    Assign {
        task_id: TaskId,
        session_id: SessionId,
        agent_id: AgentId,
        kind: AssignmentKind,
        at: Timestamp,
        reply: oneshot::Sender<Result<AssignmentId, LiveSessionError>>,
    },
    SubmitOutput {
        assignment_id: AssignmentId,
        agent_id: AgentId,
        output_hash: OutputHash,
        at: Timestamp,
        reply: oneshot::Sender<Result<(), LiveSessionError>>,
    },
    MarkApproved {
        assignment_id: AssignmentId,
        at: Timestamp,
        reply: oneshot::Sender<Result<(), LiveSessionError>>,
    },
    MarkRejected {
        assignment_id: AssignmentId,
        at: Timestamp,
        reply: oneshot::Sender<Result<(), LiveSessionError>>,
    },
    CancelAssignment {
        assignment_id: AssignmentId,
        at: Timestamp,
        reply: oneshot::Sender<Result<(), LiveSessionError>>,
    },
    GetSession {
        session_id: SessionId,
        reply: oneshot::Sender<Option<LiveSession>>,
    },
    GetAssignment {
        assignment_id: AssignmentId,
        reply: oneshot::Sender<Option<Assignment>>,
    },
    AssignmentsByTask {
        task_id: TaskId,
        reply: oneshot::Sender<Vec<Assignment>>,
    },
    AssignmentsBySession {
        session_id: SessionId,
        reply: oneshot::Sender<Vec<Assignment>>,
    },
    AssignmentsByAgent {
        agent_id: AgentId,
        reply: oneshot::Sender<Vec<Assignment>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

#[derive(Clone, Debug)]
pub struct LiveSessionHandle {
    commands: mpsc::Sender<LiveSessionCommand>,
}

impl LiveSessionHandle {
    pub async fn create_session(
        &self,
        task_id: impl Into<TaskId>,
        at: Timestamp,
    ) -> Result<SessionId, LiveSessionServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(LiveSessionCommand::CreateSession {
            task_id: task_id.into(),
            at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| LiveSessionServiceError::ResponseDropped)
    }

    pub async fn close_session(
        &self,
        session_id: impl Into<SessionId>,
        at: Timestamp,
    ) -> Result<(), LiveSessionServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(LiveSessionCommand::CloseSession {
            session_id: session_id.into(),
            at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| LiveSessionServiceError::ResponseDropped)?
            .map_err(LiveSessionServiceError::LiveSession)
    }

    pub async fn assign(
        &self,
        task_id: impl Into<TaskId>,
        session_id: impl Into<SessionId>,
        agent_id: impl Into<AgentId>,
        kind: AssignmentKind,
        at: Timestamp,
    ) -> Result<AssignmentId, LiveSessionServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(LiveSessionCommand::Assign {
            task_id: task_id.into(),
            session_id: session_id.into(),
            agent_id: agent_id.into(),
            kind,
            at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| LiveSessionServiceError::ResponseDropped)?
            .map_err(LiveSessionServiceError::LiveSession)
    }

    pub async fn submit_output(
        &self,
        assignment_id: impl Into<AssignmentId>,
        agent_id: impl Into<AgentId>,
        output_hash: impl Into<OutputHash>,
        at: Timestamp,
    ) -> Result<(), LiveSessionServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(LiveSessionCommand::SubmitOutput {
            assignment_id: assignment_id.into(),
            agent_id: agent_id.into(),
            output_hash: output_hash.into(),
            at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| LiveSessionServiceError::ResponseDropped)?
            .map_err(LiveSessionServiceError::LiveSession)
    }

    pub async fn mark_approved(
        &self,
        assignment_id: impl Into<AssignmentId>,
        at: Timestamp,
    ) -> Result<(), LiveSessionServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(LiveSessionCommand::MarkApproved {
            assignment_id: assignment_id.into(),
            at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| LiveSessionServiceError::ResponseDropped)?
            .map_err(LiveSessionServiceError::LiveSession)
    }

    pub async fn mark_rejected(
        &self,
        assignment_id: impl Into<AssignmentId>,
        at: Timestamp,
    ) -> Result<(), LiveSessionServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(LiveSessionCommand::MarkRejected {
            assignment_id: assignment_id.into(),
            at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| LiveSessionServiceError::ResponseDropped)?
            .map_err(LiveSessionServiceError::LiveSession)
    }

    pub async fn cancel_assignment(
        &self,
        assignment_id: impl Into<AssignmentId>,
        at: Timestamp,
    ) -> Result<(), LiveSessionServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(LiveSessionCommand::CancelAssignment {
            assignment_id: assignment_id.into(),
            at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| LiveSessionServiceError::ResponseDropped)?
            .map_err(LiveSessionServiceError::LiveSession)
    }

    pub async fn get_session(
        &self,
        session_id: impl Into<SessionId>,
    ) -> Result<Option<LiveSession>, LiveSessionServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(LiveSessionCommand::GetSession {
            session_id: session_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| LiveSessionServiceError::ResponseDropped)
    }

    pub async fn get_assignment(
        &self,
        assignment_id: impl Into<AssignmentId>,
    ) -> Result<Option<Assignment>, LiveSessionServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(LiveSessionCommand::GetAssignment {
            assignment_id: assignment_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| LiveSessionServiceError::ResponseDropped)
    }

    pub async fn assignments_by_task(
        &self,
        task_id: impl Into<TaskId>,
    ) -> Result<Vec<Assignment>, LiveSessionServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(LiveSessionCommand::AssignmentsByTask {
            task_id: task_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| LiveSessionServiceError::ResponseDropped)
    }

    pub async fn assignments_by_session(
        &self,
        session_id: impl Into<SessionId>,
    ) -> Result<Vec<Assignment>, LiveSessionServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(LiveSessionCommand::AssignmentsBySession {
            session_id: session_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| LiveSessionServiceError::ResponseDropped)
    }

    pub async fn assignments_by_agent(
        &self,
        agent_id: impl Into<AgentId>,
    ) -> Result<Vec<Assignment>, LiveSessionServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(LiveSessionCommand::AssignmentsByAgent {
            agent_id: agent_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| LiveSessionServiceError::ResponseDropped)
    }

    pub async fn shutdown(&self) -> Result<(), LiveSessionServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(LiveSessionCommand::Shutdown { reply }).await?;
        response
            .await
            .map_err(|_| LiveSessionServiceError::ResponseDropped)
    }

    async fn send(&self, command: LiveSessionCommand) -> Result<(), LiveSessionServiceError> {
        self.commands
            .send(command)
            .await
            .map_err(|_| LiveSessionServiceError::Stopped)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LiveSessionServiceError {
    LiveSession(LiveSessionError),
    Stopped,
    ResponseDropped,
}

impl fmt::Display for LiveSessionServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LiveSessionServiceError::LiveSession(error) => write!(f, "{error}"),
            LiveSessionServiceError::Stopped => f.write_str("live session service is stopped"),
            LiveSessionServiceError::ResponseDropped => {
                f.write_str("live session service dropped the response")
            }
        }
    }
}

impl Error for LiveSessionServiceError {}

pub struct LiveSessionService {
    core: LiveSessionCore,
    commands: mpsc::Receiver<LiveSessionCommand>,
}

impl LiveSessionService {
    pub fn spawn() -> LiveSessionHandle {
        Self::spawn_with_buffer(DEFAULT_COMMAND_BUFFER)
    }

    pub fn spawn_with_buffer(command_buffer: usize) -> LiveSessionHandle {
        assert!(
            command_buffer > 0,
            "live session command buffer must be greater than zero"
        );
        let (commands, receiver) = mpsc::channel(command_buffer);
        let service = Self {
            core: LiveSessionCore::new(),
            commands: receiver,
        };

        tokio::spawn(service.run());

        LiveSessionHandle { commands }
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

    fn handle_command(&mut self, command: LiveSessionCommand) -> Option<oneshot::Sender<()>> {
        match command {
            LiveSessionCommand::CreateSession { task_id, at, reply } => {
                let _ = reply.send(self.core.create_session(task_id, at));
                None
            }
            LiveSessionCommand::CloseSession {
                session_id,
                at,
                reply,
            } => {
                let _ = reply.send(self.core.close_session(&session_id, at));
                None
            }
            LiveSessionCommand::Assign {
                task_id,
                session_id,
                agent_id,
                kind,
                at,
                reply,
            } => {
                let _ = reply.send(self.core.assign(task_id, &session_id, agent_id, kind, at));
                None
            }
            LiveSessionCommand::SubmitOutput {
                assignment_id,
                agent_id,
                output_hash,
                at,
                reply,
            } => {
                let _ =
                    reply.send(
                        self.core
                            .submit_output(&assignment_id, agent_id, output_hash, at),
                    );
                None
            }
            LiveSessionCommand::MarkApproved {
                assignment_id,
                at,
                reply,
            } => {
                let _ = reply.send(self.core.mark_approved(&assignment_id, at));
                None
            }
            LiveSessionCommand::MarkRejected {
                assignment_id,
                at,
                reply,
            } => {
                let _ = reply.send(self.core.mark_rejected(&assignment_id, at));
                None
            }
            LiveSessionCommand::CancelAssignment {
                assignment_id,
                at,
                reply,
            } => {
                let _ = reply.send(self.core.cancel_assignment(&assignment_id, at));
                None
            }
            LiveSessionCommand::GetSession { session_id, reply } => {
                let _ = reply.send(self.core.get_session(&session_id).cloned());
                None
            }
            LiveSessionCommand::GetAssignment {
                assignment_id,
                reply,
            } => {
                let _ = reply.send(self.core.get_assignment(&assignment_id).cloned());
                None
            }
            LiveSessionCommand::AssignmentsByTask { task_id, reply } => {
                let _ = reply.send(self.core.assignments_by_task(&task_id));
                None
            }
            LiveSessionCommand::AssignmentsBySession { session_id, reply } => {
                let _ = reply.send(self.core.assignments_by_session(&session_id));
                None
            }
            LiveSessionCommand::AssignmentsByAgent { agent_id, reply } => {
                let _ = reply.send(self.core.assignments_by_agent(&agent_id));
                None
            }
            LiveSessionCommand::Shutdown { reply } => Some(reply),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn service_creates_assigns_and_submits_output() {
        let live = LiveSessionService::spawn();

        let session_id = live.create_session("task-1", Timestamp(1)).await.unwrap();
        let assignment_id = live
            .assign(
                "task-1",
                session_id.clone(),
                "executor",
                AssignmentKind::Execute,
                Timestamp(2),
            )
            .await
            .unwrap();
        live.submit_output(assignment_id.clone(), "executor", "hash-1", Timestamp(3))
            .await
            .unwrap();

        let assignment = live.get_assignment(assignment_id).await.unwrap().unwrap();
        assert_eq!(assignment.status, super::super::AssignmentStatus::Submitted);
        assert_eq!(
            live.assignments_by_session(session_id).await.unwrap().len(),
            1
        );

        live.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_returns_live_session_errors() {
        let live = LiveSessionService::spawn();

        let error = live
            .assign(
                "task-1",
                "missing",
                "executor",
                AssignmentKind::Execute,
                Timestamp(1),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error,
            LiveSessionServiceError::LiveSession(LiveSessionError::SessionNotFound(
                SessionId::from("missing")
            ))
        );

        live.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_stops_after_shutdown() {
        let live = LiveSessionService::spawn();

        live.shutdown().await.unwrap();

        assert_eq!(
            live.create_session("task-1", Timestamp(1))
                .await
                .unwrap_err(),
            LiveSessionServiceError::Stopped
        );
    }
}
