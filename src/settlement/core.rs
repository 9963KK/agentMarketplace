use std::collections::HashMap;

use crate::heartbeat::AgentId;
use crate::types::{AssignmentId, TaskId, Timestamp};

use super::types::{
    Balance, Hold, HoldId, HoldStatus, LedgerEntry, LedgerEntryKind, ReleaseEvidence,
    SettlementError,
};

#[derive(Debug, Default)]
pub struct SettlementCore {
    holds: HashMap<HoldId, Hold>,
    balances: HashMap<AgentId, Balance>,
    ledger: Vec<LedgerEntry>,
    next_hold: u64,
}

impl SettlementCore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn deposit(
        &mut self,
        agent_id: AgentId,
        amount: u64,
        at: Timestamp,
    ) -> Result<(), SettlementError> {
        if amount == 0 {
            return Err(SettlementError::ZeroAmount);
        }

        self.credit(agent_id.clone(), amount)?;
        self.ledger.push(LedgerEntry {
            hold_id: None,
            task_id: None,
            assignment_id: None,
            amount,
            kind: LedgerEntryKind::Deposited { agent_id },
            at,
        });

        Ok(())
    }

    pub fn hold(
        &mut self,
        from_agent: AgentId,
        amount: u64,
        task_id: TaskId,
        assignment_id: AssignmentId,
        agent_id: AgentId,
        at: Timestamp,
    ) -> Result<HoldId, SettlementError> {
        if amount == 0 {
            return Err(SettlementError::ZeroAmount);
        }
        let available = self.balance(&from_agent);
        if available < amount {
            return Err(SettlementError::InsufficientBalance {
                agent_id: from_agent,
                available,
                required: amount,
            });
        }

        let hold_id = self.next_hold_id();
        self.debit(from_agent.clone(), amount)?;
        self.holds.insert(
            hold_id.clone(),
            Hold {
                hold_id: hold_id.clone(),
                from_agent: from_agent.clone(),
                amount,
                task_id: task_id.clone(),
                assignment_id: assignment_id.clone(),
                agent_id: agent_id.clone(),
                status: HoldStatus::Active,
            },
        );
        self.ledger.push(LedgerEntry {
            hold_id: Some(hold_id.clone()),
            task_id: Some(task_id),
            assignment_id: Some(assignment_id),
            amount,
            kind: LedgerEntryKind::HoldCreated {
                from_agent,
                agent_id,
            },
            at,
        });

        Ok(hold_id)
    }

    pub fn release(
        &mut self,
        hold_id: &HoldId,
        evidence: ReleaseEvidence,
        at: Timestamp,
    ) -> Result<(), SettlementError> {
        let hold = self.active_hold(hold_id)?;
        validate_release_evidence(hold, &evidence)?;

        let amount = hold.amount;
        let task_id = hold.task_id.clone();
        let assignment_id = hold.assignment_id.clone();
        let to_agent = hold.agent_id.clone();
        self.credit(to_agent.clone(), amount)?;
        let hold = self
            .holds
            .get_mut(hold_id)
            .ok_or_else(|| SettlementError::HoldNotFound(hold_id.clone()))?;
        hold.status = HoldStatus::Released;
        self.ledger.push(LedgerEntry {
            hold_id: Some(hold_id.clone()),
            task_id: Some(task_id),
            assignment_id: Some(assignment_id),
            amount,
            kind: LedgerEntryKind::Released { to_agent },
            at,
        });

        Ok(())
    }

    pub fn refund(&mut self, hold_id: &HoldId, at: Timestamp) -> Result<(), SettlementError> {
        let hold = self.active_hold(hold_id)?;
        let from_agent = hold.from_agent.clone();
        let amount = hold.amount;
        let task_id = hold.task_id.clone();
        let assignment_id = hold.assignment_id.clone();

        self.credit(from_agent.clone(), amount)?;
        let hold = self
            .holds
            .get_mut(hold_id)
            .ok_or_else(|| SettlementError::HoldNotFound(hold_id.clone()))?;
        hold.status = HoldStatus::Refunded;
        self.ledger.push(LedgerEntry {
            hold_id: Some(hold_id.clone()),
            task_id: Some(task_id),
            assignment_id: Some(assignment_id),
            amount,
            kind: LedgerEntryKind::Refunded {
                to_agent: from_agent,
            },
            at,
        });

        Ok(())
    }

    pub fn balance(&self, agent_id: &AgentId) -> Balance {
        self.balances.get(agent_id).copied().unwrap_or(0)
    }

    pub fn get_hold(&self, hold_id: &HoldId) -> Option<&Hold> {
        self.holds.get(hold_id)
    }

    pub fn active_holds_for_agent(&self, agent_id: &AgentId) -> Vec<Hold> {
        self.holds
            .values()
            .filter(|hold| {
                hold.status == HoldStatus::Active
                    && (hold.from_agent == *agent_id || hold.agent_id == *agent_id)
            })
            .cloned()
            .collect()
    }

    pub fn ledger(&self) -> &[LedgerEntry] {
        &self.ledger
    }

    fn active_hold(&self, hold_id: &HoldId) -> Result<&Hold, SettlementError> {
        let hold = self
            .holds
            .get(hold_id)
            .ok_or_else(|| SettlementError::HoldNotFound(hold_id.clone()))?;
        if hold.status != HoldStatus::Active {
            return Err(SettlementError::HoldNotActive {
                hold_id: hold_id.clone(),
                status: hold.status,
            });
        }

        Ok(hold)
    }

    fn credit(&mut self, agent_id: AgentId, amount: u64) -> Result<(), SettlementError> {
        let current = self.balance(&agent_id);
        let next = current
            .checked_add(amount)
            .ok_or(SettlementError::Overflow)?;
        self.balances.insert(agent_id, next);
        Ok(())
    }

    fn debit(&mut self, agent_id: AgentId, amount: u64) -> Result<(), SettlementError> {
        let current = self.balance(&agent_id);
        let next =
            current
                .checked_sub(amount)
                .ok_or_else(|| SettlementError::InsufficientBalance {
                    agent_id: agent_id.clone(),
                    available: current,
                    required: amount,
                })?;
        self.balances.insert(agent_id, next);
        Ok(())
    }

    fn next_hold_id(&mut self) -> HoldId {
        self.next_hold += 1;
        HoldId::new(format!("hold-{}", self.next_hold))
    }
}

fn validate_release_evidence(
    hold: &Hold,
    evidence: &ReleaseEvidence,
) -> Result<(), SettlementError> {
    match evidence {
        ReleaseEvidence::AssignmentOutputAccepted {
            task_id,
            assignment_id,
            review_ids,
        } if hold.task_id == *task_id && hold.assignment_id == *assignment_id => {
            if review_ids.is_empty() {
                return Err(SettlementError::EmptyReviewEvidence);
            }
        }
        ReleaseEvidence::ReviewSubmitted {
            task_id,
            assignment_id,
            review_id: _,
        } if hold.task_id == *task_id && hold.assignment_id == *assignment_id => {}
        _ => {
            return Err(SettlementError::ReleaseEvidenceMismatch {
                hold_id: hold.hold_id.clone(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::review::ReviewId;

    use super::*;

    fn agent(id: &str) -> AgentId {
        AgentId::from(id)
    }

    fn task(id: &str) -> TaskId {
        TaskId::from(id)
    }

    fn assignment(id: &str) -> AssignmentId {
        AssignmentId::from(id)
    }

    fn review(id: &str) -> ReviewId {
        ReviewId::from(id)
    }

    fn accepted(assignment_id: &str) -> ReleaseEvidence {
        ReleaseEvidence::AssignmentOutputAccepted {
            task_id: task("task-1"),
            assignment_id: assignment(assignment_id),
            review_ids: vec![review("review-1")],
        }
    }

    fn review_submitted(assignment_id: &str) -> ReleaseEvidence {
        ReleaseEvidence::ReviewSubmitted {
            task_id: task("task-1"),
            assignment_id: assignment(assignment_id),
            review_id: review("review-1"),
        }
    }

    #[test]
    fn deposit_increases_balance_and_records_ledger_entry() {
        let mut core = SettlementCore::new();

        core.deposit(agent("publisher"), 100, Timestamp(1)).unwrap();

        assert_eq!(core.balance(&agent("publisher")), 100);
        assert_eq!(core.ledger().len(), 1);
        assert_eq!(
            core.ledger()[0].kind,
            LedgerEntryKind::Deposited {
                agent_id: agent("publisher")
            }
        );
    }

    #[test]
    fn deposit_rejects_zero_amount() {
        let mut core = SettlementCore::new();

        assert_eq!(
            core.deposit(agent("publisher"), 0, Timestamp(1))
                .unwrap_err(),
            SettlementError::ZeroAmount
        );
    }

    #[test]
    fn hold_creates_active_assignment_hold_and_debits_payer() {
        let mut core = SettlementCore::new();
        core.deposit(agent("publisher"), 100, Timestamp(0)).unwrap();

        let hold_id = core
            .hold(
                agent("publisher"),
                100,
                task("task-1"),
                assignment("execute-1"),
                agent("executor"),
                Timestamp(1),
            )
            .unwrap();

        let hold = core.get_hold(&hold_id).unwrap();
        assert_eq!(hold.status, HoldStatus::Active);
        assert_eq!(hold.amount, 100);
        assert_eq!(hold.assignment_id, assignment("execute-1"));
        assert_eq!(hold.agent_id, agent("executor"));
        assert_eq!(core.balance(&agent("publisher")), 0);
        assert_eq!(core.ledger().len(), 2);
    }

    #[test]
    fn hold_rejects_zero_amount() {
        let mut core = SettlementCore::new();

        assert_eq!(
            core.hold(
                agent("publisher"),
                0,
                task("task-1"),
                assignment("execute-1"),
                agent("executor"),
                Timestamp(1),
            )
            .unwrap_err(),
            SettlementError::ZeroAmount
        );
    }

    #[test]
    fn hold_rejects_insufficient_balance_without_creating_hold() {
        let mut core = SettlementCore::new();
        core.deposit(agent("publisher"), 99, Timestamp(0)).unwrap();

        assert_eq!(
            core.hold(
                agent("publisher"),
                100,
                task("task-1"),
                assignment("execute-1"),
                agent("executor"),
                Timestamp(1),
            )
            .unwrap_err(),
            SettlementError::InsufficientBalance {
                agent_id: agent("publisher"),
                available: 99,
                required: 100
            }
        );
        assert_eq!(core.balance(&agent("publisher")), 99);
        assert_eq!(core.ledger().len(), 1);
    }

    #[test]
    fn release_assignment_hold_credits_bound_agent() {
        let mut core = SettlementCore::new();
        core.deposit(agent("publisher"), 100, Timestamp(0)).unwrap();
        let hold_id = core
            .hold(
                agent("publisher"),
                100,
                task("task-1"),
                assignment("execute-1"),
                agent("executor"),
                Timestamp(1),
            )
            .unwrap();

        core.release(&hold_id, accepted("execute-1"), Timestamp(2))
            .unwrap();

        assert_eq!(core.balance(&agent("executor")), 100);
        assert_eq!(core.balance(&agent("publisher")), 0);
        assert_eq!(
            core.get_hold(&hold_id).unwrap().status,
            HoldStatus::Released
        );
        assert_eq!(core.ledger().len(), 3);
    }

    #[test]
    fn release_review_hold_accepts_review_submitted_evidence() {
        let mut core = SettlementCore::new();
        core.deposit(agent("publisher"), 25, Timestamp(0)).unwrap();
        let hold_id = core
            .hold(
                agent("publisher"),
                25,
                task("task-1"),
                assignment("review-assignment-1"),
                agent("reviewer-1"),
                Timestamp(1),
            )
            .unwrap();

        core.release(
            &hold_id,
            review_submitted("review-assignment-1"),
            Timestamp(2),
        )
        .unwrap();

        assert_eq!(core.balance(&agent("reviewer-1")), 25);
        assert_eq!(core.balance(&agent("publisher")), 0);
    }

    #[test]
    fn release_rejects_wrong_assignment_or_empty_review_evidence() {
        let mut core = SettlementCore::new();
        core.deposit(agent("publisher"), 100, Timestamp(0)).unwrap();
        let hold_id = core
            .hold(
                agent("publisher"),
                100,
                task("task-1"),
                assignment("execute-1"),
                agent("executor"),
                Timestamp(1),
            )
            .unwrap();

        assert_eq!(
            core.release(&hold_id, accepted("execute-2"), Timestamp(2))
                .unwrap_err(),
            SettlementError::ReleaseEvidenceMismatch {
                hold_id: hold_id.clone()
            }
        );

        assert_eq!(
            core.release(
                &hold_id,
                ReleaseEvidence::AssignmentOutputAccepted {
                    task_id: task("task-1"),
                    assignment_id: assignment("execute-1"),
                    review_ids: Vec::new(),
                },
                Timestamp(2),
            )
            .unwrap_err(),
            SettlementError::EmptyReviewEvidence
        );
    }

    #[test]
    fn refund_returns_amount_to_payer() {
        let mut core = SettlementCore::new();
        core.deposit(agent("publisher"), 100, Timestamp(0)).unwrap();
        let hold_id = core
            .hold(
                agent("publisher"),
                100,
                task("task-1"),
                assignment("execute-1"),
                agent("executor"),
                Timestamp(1),
            )
            .unwrap();

        core.refund(&hold_id, Timestamp(2)).unwrap();

        assert_eq!(core.balance(&agent("publisher")), 100);
        assert_eq!(
            core.get_hold(&hold_id).unwrap().status,
            HoldStatus::Refunded
        );
    }

    #[test]
    fn released_or_refunded_hold_cannot_change_again() {
        let mut core = SettlementCore::new();
        core.deposit(agent("publisher"), 100, Timestamp(0)).unwrap();
        let released = core
            .hold(
                agent("publisher"),
                100,
                task("task-1"),
                assignment("execute-1"),
                agent("executor"),
                Timestamp(1),
            )
            .unwrap();
        core.release(&released, accepted("execute-1"), Timestamp(2))
            .unwrap();

        assert_eq!(
            core.refund(&released, Timestamp(3)).unwrap_err(),
            SettlementError::HoldNotActive {
                hold_id: released,
                status: HoldStatus::Released,
            }
        );
    }

    #[test]
    fn active_holds_for_agent_returns_payer_and_payee_holds() {
        let mut core = SettlementCore::new();
        core.deposit(agent("publisher"), 125, Timestamp(0)).unwrap();
        core.hold(
            agent("publisher"),
            100,
            task("task-1"),
            assignment("execute-1"),
            agent("executor"),
            Timestamp(1),
        )
        .unwrap();
        core.hold(
            agent("publisher"),
            25,
            task("task-1"),
            assignment("review-1"),
            agent("reviewer-1"),
            Timestamp(1),
        )
        .unwrap();

        assert_eq!(core.active_holds_for_agent(&agent("publisher")).len(), 2);
        assert_eq!(core.active_holds_for_agent(&agent("executor")).len(), 1);
        assert_eq!(core.active_holds_for_agent(&agent("reviewer-1")).len(), 1);
    }
}
