use std::error::Error;
use std::fmt;

use tokio::sync::{mpsc, oneshot};

use crate::heartbeat::AgentId;
use crate::types::Timestamp;

use super::SettlementCore;
use super::types::{
    Balance, Hold, HoldId, HoldRequest, LedgerEntry, ReleaseEvidence, SettlementError,
};

const DEFAULT_COMMAND_BUFFER: usize = 128;

#[derive(Debug)]
pub enum SettlementCommand {
    Deposit {
        agent_id: AgentId,
        amount: u64,
        at: Timestamp,
        reply: oneshot::Sender<Result<(), SettlementError>>,
    },
    Hold {
        request: HoldRequest,
        at: Timestamp,
        reply: oneshot::Sender<Result<HoldId, SettlementError>>,
    },
    Release {
        hold_id: HoldId,
        evidence: ReleaseEvidence,
        at: Timestamp,
        reply: oneshot::Sender<Result<(), SettlementError>>,
    },
    Refund {
        hold_id: HoldId,
        at: Timestamp,
        reply: oneshot::Sender<Result<(), SettlementError>>,
    },
    Balance {
        agent_id: AgentId,
        reply: oneshot::Sender<Balance>,
    },
    GetHold {
        hold_id: HoldId,
        reply: oneshot::Sender<Option<Hold>>,
    },
    ActiveHoldsForAgent {
        agent_id: AgentId,
        reply: oneshot::Sender<Vec<Hold>>,
    },
    Ledger {
        reply: oneshot::Sender<Vec<LedgerEntry>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

#[derive(Clone, Debug)]
pub struct SettlementHandle {
    commands: mpsc::Sender<SettlementCommand>,
}

impl SettlementHandle {
    pub async fn deposit(
        &self,
        agent_id: impl Into<AgentId>,
        amount: u64,
        at: Timestamp,
    ) -> Result<(), SettlementServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(SettlementCommand::Deposit {
            agent_id: agent_id.into(),
            amount,
            at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| SettlementServiceError::ResponseDropped)?
            .map_err(SettlementServiceError::Settlement)
    }

    pub async fn hold(
        &self,
        request: HoldRequest,
        at: Timestamp,
    ) -> Result<HoldId, SettlementServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(SettlementCommand::Hold { request, at, reply })
            .await?;
        response
            .await
            .map_err(|_| SettlementServiceError::ResponseDropped)?
            .map_err(SettlementServiceError::Settlement)
    }

    pub(crate) async fn release(
        &self,
        hold_id: impl Into<HoldId>,
        evidence: ReleaseEvidence,
        at: Timestamp,
    ) -> Result<(), SettlementServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(SettlementCommand::Release {
            hold_id: hold_id.into(),
            evidence,
            at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| SettlementServiceError::ResponseDropped)?
            .map_err(SettlementServiceError::Settlement)
    }

    pub async fn refund(
        &self,
        hold_id: impl Into<HoldId>,
        at: Timestamp,
    ) -> Result<(), SettlementServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(SettlementCommand::Refund {
            hold_id: hold_id.into(),
            at,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| SettlementServiceError::ResponseDropped)?
            .map_err(SettlementServiceError::Settlement)
    }

    pub async fn balance(
        &self,
        agent_id: impl Into<AgentId>,
    ) -> Result<Balance, SettlementServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(SettlementCommand::Balance {
            agent_id: agent_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| SettlementServiceError::ResponseDropped)
    }

    pub async fn get_hold(
        &self,
        hold_id: impl Into<HoldId>,
    ) -> Result<Option<Hold>, SettlementServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(SettlementCommand::GetHold {
            hold_id: hold_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| SettlementServiceError::ResponseDropped)
    }

    pub async fn active_holds_for_agent(
        &self,
        agent_id: impl Into<AgentId>,
    ) -> Result<Vec<Hold>, SettlementServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(SettlementCommand::ActiveHoldsForAgent {
            agent_id: agent_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| SettlementServiceError::ResponseDropped)
    }

    pub async fn ledger(&self) -> Result<Vec<LedgerEntry>, SettlementServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(SettlementCommand::Ledger { reply }).await?;
        response
            .await
            .map_err(|_| SettlementServiceError::ResponseDropped)
    }

    pub async fn shutdown(&self) -> Result<(), SettlementServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(SettlementCommand::Shutdown { reply }).await?;
        response
            .await
            .map_err(|_| SettlementServiceError::ResponseDropped)
    }

    async fn send(&self, command: SettlementCommand) -> Result<(), SettlementServiceError> {
        self.commands
            .send(command)
            .await
            .map_err(|_| SettlementServiceError::Stopped)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettlementServiceError {
    Settlement(SettlementError),
    Stopped,
    ResponseDropped,
}

impl fmt::Display for SettlementServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettlementServiceError::Settlement(error) => write!(f, "{error}"),
            SettlementServiceError::Stopped => f.write_str("settlement service is stopped"),
            SettlementServiceError::ResponseDropped => {
                f.write_str("settlement service dropped the response")
            }
        }
    }
}

impl Error for SettlementServiceError {}

pub struct SettlementService {
    core: SettlementCore,
    commands: mpsc::Receiver<SettlementCommand>,
}

impl SettlementService {
    pub fn spawn() -> SettlementHandle {
        Self::spawn_with_buffer(DEFAULT_COMMAND_BUFFER)
    }

    pub fn spawn_with_buffer(command_buffer: usize) -> SettlementHandle {
        assert!(
            command_buffer > 0,
            "settlement command buffer must be greater than zero"
        );
        let (commands, receiver) = mpsc::channel(command_buffer);
        let service = Self {
            core: SettlementCore::new(),
            commands: receiver,
        };

        tokio::spawn(service.run());

        SettlementHandle { commands }
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

    fn handle_command(&mut self, command: SettlementCommand) -> Option<oneshot::Sender<()>> {
        match command {
            SettlementCommand::Deposit {
                agent_id,
                amount,
                at,
                reply,
            } => {
                let _ = reply.send(self.core.deposit(agent_id, amount, at));
                None
            }
            SettlementCommand::Hold { request, at, reply } => {
                let _ = reply.send(self.core.hold(request, at));
                None
            }
            SettlementCommand::Release {
                hold_id,
                evidence,
                at,
                reply,
            } => {
                let _ = reply.send(self.core.release(&hold_id, evidence, at));
                None
            }
            SettlementCommand::Refund { hold_id, at, reply } => {
                let _ = reply.send(self.core.refund(&hold_id, at));
                None
            }
            SettlementCommand::Balance { agent_id, reply } => {
                let _ = reply.send(self.core.balance(&agent_id));
                None
            }
            SettlementCommand::GetHold { hold_id, reply } => {
                let _ = reply.send(self.core.get_hold(&hold_id).cloned());
                None
            }
            SettlementCommand::ActiveHoldsForAgent { agent_id, reply } => {
                let _ = reply.send(self.core.active_holds_for_agent(&agent_id));
                None
            }
            SettlementCommand::Ledger { reply } => {
                let _ = reply.send(self.core.ledger().to_vec());
                None
            }
            SettlementCommand::Shutdown { reply } => Some(reply),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::review::ReviewId;
    use crate::settlement::HoldKind;
    use crate::types::{AssignmentId, TaskId};

    use super::*;

    fn accepted(assignment_id: &str) -> ReleaseEvidence {
        ReleaseEvidence::AssignmentOutputAccepted {
            task_id: TaskId::from("task-1"),
            assignment_id: AssignmentId::from(assignment_id),
            review_ids: vec![ReviewId::from("review-1")],
        }
    }

    fn hold_request(amount: u64) -> HoldRequest {
        HoldRequest::new(
            AgentId::from("publisher"),
            amount,
            TaskId::from("task-1"),
            AssignmentId::from("execute-1"),
            AgentId::from("executor"),
            HoldKind::Execute,
        )
    }

    #[tokio::test]
    async fn service_holds_releases_and_reports_balance() {
        let settlement = SettlementService::spawn();

        settlement
            .deposit("publisher", 100, Timestamp(0))
            .await
            .unwrap();
        let hold_id = settlement
            .hold(hold_request(100), Timestamp(1))
            .await
            .unwrap();
        settlement
            .release(hold_id.clone(), accepted("execute-1"), Timestamp(2))
            .await
            .unwrap();

        assert_eq!(settlement.balance("executor").await.unwrap(), 100);
        assert_eq!(settlement.balance("publisher").await.unwrap(), 0);
        assert_eq!(
            settlement.get_hold(hold_id).await.unwrap().unwrap().status,
            crate::settlement::HoldStatus::Released
        );
        assert_eq!(settlement.ledger().await.unwrap().len(), 3);

        settlement.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_deposits_and_rejects_insufficient_hold_balance() {
        let settlement = SettlementService::spawn();

        settlement
            .deposit("publisher", 99, Timestamp(0))
            .await
            .unwrap();

        let error = settlement
            .hold(hold_request(100), Timestamp(1))
            .await
            .unwrap_err();

        assert_eq!(
            error,
            SettlementServiceError::Settlement(SettlementError::InsufficientBalance {
                agent_id: AgentId::from("publisher"),
                available: 99,
                required: 100
            })
        );
        assert_eq!(settlement.balance("publisher").await.unwrap(), 99);

        settlement.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_returns_settlement_errors() {
        let settlement = SettlementService::spawn();

        let error = settlement
            .hold(hold_request(0), Timestamp(1))
            .await
            .unwrap_err();

        assert_eq!(
            error,
            SettlementServiceError::Settlement(SettlementError::ZeroAmount)
        );

        settlement.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_stops_after_shutdown() {
        let settlement = SettlementService::spawn();

        settlement.shutdown().await.unwrap();

        assert_eq!(
            settlement
                .hold(hold_request(100), Timestamp(1))
                .await
                .unwrap_err(),
            SettlementServiceError::Stopped
        );
    }
}
