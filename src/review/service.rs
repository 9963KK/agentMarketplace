use std::error::Error;
use std::fmt;

use tokio::sync::{mpsc, oneshot};

use crate::types::{AssignmentId, TaskId, Timestamp};

use super::ReviewCore;
use super::types::{ReviewCriteria, ReviewError, ReviewId, ReviewSession, Verdict, VerdictRecord};

const DEFAULT_COMMAND_BUFFER: usize = 128;

#[derive(Debug)]
pub enum ReviewCommand {
    Request {
        task_id: TaskId,
        target_assignment_id: AssignmentId,
        review_assignment_ids: Vec<AssignmentId>,
        criteria: ReviewCriteria,
        created_at: Timestamp,
        reply: oneshot::Sender<Result<ReviewId, ReviewError>>,
    },
    Submit {
        review_id: ReviewId,
        review_assignment_id: AssignmentId,
        verdict: Verdict,
        submitted_at: Timestamp,
        reply: oneshot::Sender<Result<(), ReviewError>>,
    },
    Collect {
        review_id: ReviewId,
        reply: oneshot::Sender<Option<Vec<VerdictRecord>>>,
    },
    CollectByAssignment {
        assignment_id: AssignmentId,
        reply: oneshot::Sender<Vec<ReviewSession>>,
    },
    CollectByTask {
        task_id: TaskId,
        reply: oneshot::Sender<Vec<ReviewSession>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

#[derive(Clone, Debug)]
pub struct ReviewHandle {
    commands: mpsc::Sender<ReviewCommand>,
}

impl ReviewHandle {
    pub async fn request(
        &self,
        task_id: impl Into<TaskId>,
        target_assignment_id: impl Into<AssignmentId>,
        review_assignment_ids: Vec<AssignmentId>,
        criteria: ReviewCriteria,
        created_at: Timestamp,
    ) -> Result<ReviewId, ReviewServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ReviewCommand::Request {
            task_id: task_id.into(),
            target_assignment_id: target_assignment_id.into(),
            review_assignment_ids,
            criteria,
            created_at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| ReviewServiceError::ResponseDropped)?
            .map_err(ReviewServiceError::Review)
    }

    pub async fn submit(
        &self,
        review_id: impl Into<ReviewId>,
        review_assignment_id: impl Into<AssignmentId>,
        verdict: Verdict,
        submitted_at: Timestamp,
    ) -> Result<(), ReviewServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ReviewCommand::Submit {
            review_id: review_id.into(),
            review_assignment_id: review_assignment_id.into(),
            verdict,
            submitted_at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| ReviewServiceError::ResponseDropped)?
            .map_err(ReviewServiceError::Review)
    }

    pub async fn collect(
        &self,
        review_id: impl Into<ReviewId>,
    ) -> Result<Option<Vec<VerdictRecord>>, ReviewServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ReviewCommand::Collect {
            review_id: review_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| ReviewServiceError::ResponseDropped)
    }

    pub async fn collect_by_assignment(
        &self,
        assignment_id: impl Into<AssignmentId>,
    ) -> Result<Vec<ReviewSession>, ReviewServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ReviewCommand::CollectByAssignment {
            assignment_id: assignment_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| ReviewServiceError::ResponseDropped)
    }

    pub async fn collect_by_task(
        &self,
        task_id: impl Into<TaskId>,
    ) -> Result<Vec<ReviewSession>, ReviewServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ReviewCommand::CollectByTask {
            task_id: task_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| ReviewServiceError::ResponseDropped)
    }

    pub async fn shutdown(&self) -> Result<(), ReviewServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(ReviewCommand::Shutdown { reply }).await?;
        response
            .await
            .map_err(|_| ReviewServiceError::ResponseDropped)
    }

    async fn send(&self, command: ReviewCommand) -> Result<(), ReviewServiceError> {
        self.commands
            .send(command)
            .await
            .map_err(|_| ReviewServiceError::Stopped)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewServiceError {
    Review(ReviewError),
    Stopped,
    ResponseDropped,
}

impl fmt::Display for ReviewServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReviewServiceError::Review(error) => write!(f, "{error}"),
            ReviewServiceError::Stopped => f.write_str("review service is stopped"),
            ReviewServiceError::ResponseDropped => {
                f.write_str("review service dropped the response")
            }
        }
    }
}

impl Error for ReviewServiceError {}

pub struct ReviewService {
    core: ReviewCore,
    commands: mpsc::Receiver<ReviewCommand>,
}

impl ReviewService {
    pub fn spawn() -> ReviewHandle {
        Self::spawn_with_buffer(DEFAULT_COMMAND_BUFFER)
    }

    pub fn spawn_with_buffer(command_buffer: usize) -> ReviewHandle {
        assert!(
            command_buffer > 0,
            "review command buffer must be greater than zero"
        );
        let (commands, receiver) = mpsc::channel(command_buffer);
        let service = Self {
            core: ReviewCore::new(),
            commands: receiver,
        };

        tokio::spawn(service.run());

        ReviewHandle { commands }
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

    fn handle_command(&mut self, command: ReviewCommand) -> Option<oneshot::Sender<()>> {
        match command {
            ReviewCommand::Request {
                task_id,
                target_assignment_id,
                review_assignment_ids,
                criteria,
                created_at,
                reply,
            } => {
                let _ = reply.send(self.core.request(
                    task_id,
                    target_assignment_id,
                    review_assignment_ids,
                    criteria,
                    created_at,
                ));
                None
            }
            ReviewCommand::Submit {
                review_id,
                review_assignment_id,
                verdict,
                submitted_at,
                reply,
            } => {
                let _ = reply.send(self.core.submit(
                    &review_id,
                    review_assignment_id,
                    verdict,
                    submitted_at,
                ));
                None
            }
            ReviewCommand::Collect { review_id, reply } => {
                let _ = reply.send(self.core.collect(&review_id));
                None
            }
            ReviewCommand::CollectByAssignment {
                assignment_id,
                reply,
            } => {
                let _ = reply.send(self.core.collect_by_assignment(&assignment_id));
                None
            }
            ReviewCommand::CollectByTask { task_id, reply } => {
                let _ = reply.send(self.core.collect_by_task(&task_id));
                None
            }
            ReviewCommand::Shutdown { reply } => Some(reply),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::review::{MAX_CRITERIA_BYTES, VerdictKind};

    use super::*;

    fn assignment(id: &str) -> AssignmentId {
        AssignmentId::from(id)
    }

    fn verdict(kind: VerdictKind, score_bps: u16) -> Verdict {
        Verdict {
            kind,
            score_bps,
            feedback: "reviewed".to_string(),
        }
    }

    #[tokio::test]
    async fn service_requests_submits_and_collects_verdicts() {
        let review = ReviewService::spawn();

        let review_id = review
            .request(
                "task-1",
                "execute-1",
                vec![assignment("review-1")],
                ReviewCriteria::plain_text("check output"),
                Timestamp(1),
            )
            .await
            .unwrap();
        review
            .submit(
                review_id.clone(),
                "review-1",
                verdict(VerdictKind::Passed, 9_000),
                Timestamp(2),
            )
            .await
            .unwrap();

        let verdicts = review.collect(review_id.clone()).await.unwrap().unwrap();
        let by_task = review.collect_by_task("task-1").await.unwrap();
        let by_assignment = review.collect_by_assignment("execute-1").await.unwrap();

        assert_eq!(verdicts.len(), 1);
        assert_eq!(by_task.len(), 1);
        assert_eq!(by_assignment.len(), 1);
        assert_eq!(by_task[0].review_id, review_id);
        assert_eq!(by_task[0].target_assignment_id, assignment("execute-1"));

        review.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_returns_review_errors() {
        let review = ReviewService::spawn();

        let error = review
            .request(
                "task-1",
                "execute-1",
                vec![assignment("review-1")],
                ReviewCriteria::plain_text("x".repeat(MAX_CRITERIA_BYTES + 1)),
                Timestamp(1),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error,
            ReviewServiceError::Review(ReviewError::CriteriaTooLarge {
                max_bytes: MAX_CRITERIA_BYTES,
                actual_bytes: MAX_CRITERIA_BYTES + 1,
            })
        );

        review.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_stops_after_shutdown() {
        let review = ReviewService::spawn();

        review.shutdown().await.unwrap();

        assert_eq!(
            review
                .request(
                    "task-1",
                    "execute-1",
                    Vec::new(),
                    ReviewCriteria::plain_text("check output"),
                    Timestamp(1),
                )
                .await
                .unwrap_err(),
            ReviewServiceError::Stopped
        );
    }
}
