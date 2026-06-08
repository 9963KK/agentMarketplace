use std::error::Error;
use std::fmt;

use tokio::sync::{mpsc, oneshot};

use crate::heartbeat::AgentId;
use crate::types::{TaskId, Timestamp};

use super::TaskCore;
use super::types::{Task, TaskError};

const DEFAULT_COMMAND_BUFFER: usize = 128;

#[derive(Debug)]
pub enum TaskCommand {
    Create {
        publisher: AgentId,
        created_at: Timestamp,
        reply: oneshot::Sender<Result<TaskId, TaskError>>,
    },
    AddParticipant {
        task_id: TaskId,
        agent_id: AgentId,
        updated_at: Timestamp,
        reply: oneshot::Sender<Result<(), TaskError>>,
    },
    RemoveParticipant {
        task_id: TaskId,
        agent_id: AgentId,
        updated_at: Timestamp,
        reply: oneshot::Sender<Result<bool, TaskError>>,
    },
    Complete {
        task_id: TaskId,
        completed_at: Timestamp,
        reply: oneshot::Sender<Result<(), TaskError>>,
    },
    Cancel {
        task_id: TaskId,
        cancelled_at: Timestamp,
        reply: oneshot::Sender<Result<(), TaskError>>,
    },
    Get {
        task_id: TaskId,
        reply: oneshot::Sender<Option<Task>>,
    },
    ActiveTasksByAgent {
        agent_id: AgentId,
        reply: oneshot::Sender<Vec<Task>>,
    },
    TaskHistoryByAgent {
        agent_id: AgentId,
        reply: oneshot::Sender<Vec<Task>>,
    },
    TasksByPublisher {
        agent_id: AgentId,
        reply: oneshot::Sender<Vec<Task>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

#[derive(Clone, Debug)]
pub struct TaskHandle {
    commands: mpsc::Sender<TaskCommand>,
}

impl TaskHandle {
    pub async fn create(
        &self,
        publisher: impl Into<AgentId>,
        created_at: Timestamp,
    ) -> Result<TaskId, TaskServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(TaskCommand::Create {
            publisher: publisher.into(),
            created_at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| TaskServiceError::ResponseDropped)?
            .map_err(TaskServiceError::Task)
    }

    pub async fn add_participant(
        &self,
        task_id: impl Into<TaskId>,
        agent_id: impl Into<AgentId>,
        updated_at: Timestamp,
    ) -> Result<(), TaskServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(TaskCommand::AddParticipant {
            task_id: task_id.into(),
            agent_id: agent_id.into(),
            updated_at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| TaskServiceError::ResponseDropped)?
            .map_err(TaskServiceError::Task)
    }

    pub async fn remove_participant(
        &self,
        task_id: impl Into<TaskId>,
        agent_id: impl Into<AgentId>,
        updated_at: Timestamp,
    ) -> Result<bool, TaskServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(TaskCommand::RemoveParticipant {
            task_id: task_id.into(),
            agent_id: agent_id.into(),
            updated_at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| TaskServiceError::ResponseDropped)?
            .map_err(TaskServiceError::Task)
    }

    pub async fn complete(
        &self,
        task_id: impl Into<TaskId>,
        completed_at: Timestamp,
    ) -> Result<(), TaskServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(TaskCommand::Complete {
            task_id: task_id.into(),
            completed_at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| TaskServiceError::ResponseDropped)?
            .map_err(TaskServiceError::Task)
    }

    pub async fn cancel(
        &self,
        task_id: impl Into<TaskId>,
        cancelled_at: Timestamp,
    ) -> Result<(), TaskServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(TaskCommand::Cancel {
            task_id: task_id.into(),
            cancelled_at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| TaskServiceError::ResponseDropped)?
            .map_err(TaskServiceError::Task)
    }

    pub async fn get(&self, task_id: impl Into<TaskId>) -> Result<Option<Task>, TaskServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(TaskCommand::Get {
            task_id: task_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| TaskServiceError::ResponseDropped)
    }

    pub async fn active_tasks_by_agent(
        &self,
        agent_id: impl Into<AgentId>,
    ) -> Result<Vec<Task>, TaskServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(TaskCommand::ActiveTasksByAgent {
            agent_id: agent_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| TaskServiceError::ResponseDropped)
    }

    pub async fn task_history_by_agent(
        &self,
        agent_id: impl Into<AgentId>,
    ) -> Result<Vec<Task>, TaskServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(TaskCommand::TaskHistoryByAgent {
            agent_id: agent_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| TaskServiceError::ResponseDropped)
    }

    pub async fn tasks_by_publisher(
        &self,
        agent_id: impl Into<AgentId>,
    ) -> Result<Vec<Task>, TaskServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(TaskCommand::TasksByPublisher {
            agent_id: agent_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| TaskServiceError::ResponseDropped)
    }

    pub async fn shutdown(&self) -> Result<(), TaskServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(TaskCommand::Shutdown { reply }).await?;
        response
            .await
            .map_err(|_| TaskServiceError::ResponseDropped)
    }

    async fn send(&self, command: TaskCommand) -> Result<(), TaskServiceError> {
        self.commands
            .send(command)
            .await
            .map_err(|_| TaskServiceError::Stopped)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TaskServiceError {
    Task(TaskError),
    Stopped,
    ResponseDropped,
}

impl fmt::Display for TaskServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskServiceError::Task(error) => write!(f, "{error}"),
            TaskServiceError::Stopped => f.write_str("task service is stopped"),
            TaskServiceError::ResponseDropped => f.write_str("task service dropped the response"),
        }
    }
}

impl Error for TaskServiceError {}

pub struct TaskService {
    core: TaskCore,
    commands: mpsc::Receiver<TaskCommand>,
}

impl TaskService {
    pub fn spawn() -> TaskHandle {
        Self::spawn_with_buffer(DEFAULT_COMMAND_BUFFER)
    }

    pub fn spawn_with_buffer(command_buffer: usize) -> TaskHandle {
        assert!(
            command_buffer > 0,
            "task command buffer must be greater than zero"
        );
        let (commands, receiver) = mpsc::channel(command_buffer);
        let service = Self {
            core: TaskCore::new(),
            commands: receiver,
        };

        tokio::spawn(service.run());

        TaskHandle { commands }
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

    fn handle_command(&mut self, command: TaskCommand) -> Option<oneshot::Sender<()>> {
        match command {
            TaskCommand::Create {
                publisher,
                created_at,
                reply,
            } => {
                let _ = reply.send(self.core.create(publisher, created_at));
                None
            }
            TaskCommand::AddParticipant {
                task_id,
                agent_id,
                updated_at,
                reply,
            } => {
                let _ = reply.send(self.core.add_participant(&task_id, agent_id, updated_at));
                None
            }
            TaskCommand::RemoveParticipant {
                task_id,
                agent_id,
                updated_at,
                reply,
            } => {
                let _ = reply.send(
                    self.core
                        .remove_participant(&task_id, &agent_id, updated_at),
                );
                None
            }
            TaskCommand::Complete {
                task_id,
                completed_at,
                reply,
            } => {
                let _ = reply.send(self.core.complete(&task_id, completed_at));
                None
            }
            TaskCommand::Cancel {
                task_id,
                cancelled_at,
                reply,
            } => {
                let _ = reply.send(self.core.cancel(&task_id, cancelled_at));
                None
            }
            TaskCommand::Get { task_id, reply } => {
                let _ = reply.send(self.core.get(&task_id).cloned());
                None
            }
            TaskCommand::ActiveTasksByAgent { agent_id, reply } => {
                let _ = reply.send(self.core.active_tasks_by_agent(&agent_id));
                None
            }
            TaskCommand::TaskHistoryByAgent { agent_id, reply } => {
                let _ = reply.send(self.core.task_history_by_agent(&agent_id));
                None
            }
            TaskCommand::TasksByPublisher { agent_id, reply } => {
                let _ = reply.send(self.core.tasks_by_publisher(&agent_id));
                None
            }
            TaskCommand::Shutdown { reply } => Some(reply),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::task::TaskStatus;

    use super::*;

    #[tokio::test]
    async fn service_creates_updates_and_queries_task() {
        let task = TaskService::spawn();

        let task_id = task.create("publisher", Timestamp(1)).await.unwrap();
        task.add_participant(task_id.clone(), "worker", Timestamp(2))
            .await
            .unwrap();

        assert_eq!(
            task.get(task_id.clone()).await.unwrap().unwrap().status,
            TaskStatus::Active
        );
        assert_eq!(
            task.active_tasks_by_agent("worker").await.unwrap()[0].task_id,
            task_id
        );
        assert_eq!(task.tasks_by_publisher("publisher").await.unwrap().len(), 1);

        task.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_removes_participant_but_keeps_history() {
        let task = TaskService::spawn();

        let task_id = task.create("publisher", Timestamp(1)).await.unwrap();
        task.add_participant(task_id.clone(), "worker", Timestamp(2))
            .await
            .unwrap();
        assert!(
            task.remove_participant(task_id.clone(), "worker", Timestamp(3))
                .await
                .unwrap()
        );

        assert!(
            task.active_tasks_by_agent("worker")
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(task.task_history_by_agent("worker").await.unwrap().len(), 1);

        task.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_completes_and_cancels_tasks() {
        let task = TaskService::spawn();

        let completed = task.create("publisher", Timestamp(1)).await.unwrap();
        let cancelled = task.create("publisher", Timestamp(1)).await.unwrap();

        task.complete(completed.clone(), Timestamp(2))
            .await
            .unwrap();
        task.cancel(cancelled.clone(), Timestamp(2)).await.unwrap();

        assert_eq!(
            task.get(completed).await.unwrap().unwrap().status,
            TaskStatus::Completed
        );
        assert_eq!(
            task.get(cancelled).await.unwrap().unwrap().status,
            TaskStatus::Cancelled
        );

        task.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_returns_task_errors() {
        let task = TaskService::spawn();

        let error = task
            .add_participant("missing", "worker", Timestamp(1))
            .await
            .unwrap_err();

        assert_eq!(
            error,
            TaskServiceError::Task(TaskError::TaskNotFound(TaskId::from("missing")))
        );

        task.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_stops_after_shutdown() {
        let task = TaskService::spawn();

        task.shutdown().await.unwrap();

        assert_eq!(
            task.create("publisher", Timestamp(1)).await.unwrap_err(),
            TaskServiceError::Stopped
        );
    }
}
