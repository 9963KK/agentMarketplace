mod clock;
mod core;
mod types;

pub use clock::{FixedRuntimeClock, RuntimeClock, SystemRuntimeClock};
pub use core::{Runtime, RuntimeHeartbeatSink};
pub use types::{RuntimeAction, RuntimeActionError, RuntimeActionKind, RuntimeEventReport};

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::heartbeat::{AgentId, HeartbeatConfig, HeartbeatEvent, HeartbeatService};
    use crate::livesession::{AssignmentKind, AssignmentStatus, LiveSessionService};
    use crate::registry::{AgentIdentity, Capability, DiscoveryQuery, RegistryService};
    use crate::settlement::{HoldStatus, SettlementService};
    use crate::task::TaskService;
    use crate::types::Timestamp;

    use super::{FixedRuntimeClock, Runtime, RuntimeAction};

    #[tokio::test]
    async fn timeout_event_cleans_runtime_state_without_orchestrating_business() {
        let registry = RegistryService::spawn();
        let settlement = SettlementService::spawn();
        let live_sessions = LiveSessionService::spawn();
        let tasks = TaskService::spawn();
        let runtime = Runtime::new(
            registry.clone(),
            settlement.clone(),
            live_sessions.clone(),
            tasks.clone(),
        );

        registry
            .register(AgentIdentity::new(AgentId::from("executor")))
            .await
            .unwrap();
        registry
            .declare_capabilities("executor", vec![Capability::new("rust", 2)])
            .await
            .unwrap();
        registry.mark_alive("executor").await.unwrap();

        let task_id = tasks.create("publisher", Timestamp(1)).await.unwrap();
        tasks
            .add_participant(task_id.clone(), "executor", Timestamp(2))
            .await
            .unwrap();
        let session_id = live_sessions
            .create_session(task_id.clone(), Timestamp(3))
            .await
            .unwrap();
        let assignment_id = live_sessions
            .assign(
                task_id.clone(),
                session_id,
                "executor",
                AssignmentKind::Execute,
                Timestamp(4),
            )
            .await
            .unwrap();
        settlement
            .deposit("publisher", 100, Timestamp(5))
            .await
            .unwrap();
        let hold_id = settlement
            .hold(
                "publisher",
                100,
                task_id.clone(),
                assignment_id.clone(),
                "executor",
                Timestamp(6),
            )
            .await
            .unwrap();

        let report = runtime
            .handle_heartbeat_event_at(
                HeartbeatEvent::AgentTimedOut {
                    agent_id: AgentId::from("executor"),
                },
                Timestamp(7),
            )
            .await;

        assert!(!report.has_errors());
        assert!(
            report
                .actions
                .contains(&RuntimeAction::RegistryMarkedTimedOut {
                    agent_id: AgentId::from("executor")
                })
        );
        assert!(report.actions.contains(&RuntimeAction::HoldRefunded {
            hold_id: hold_id.clone()
        }));
        assert!(
            report
                .actions
                .contains(&RuntimeAction::AssignmentCancelled {
                    assignment_id: assignment_id.clone()
                })
        );
        assert!(
            report
                .actions
                .contains(&RuntimeAction::TaskParticipantRemoved {
                    task_id: task_id.clone(),
                    agent_id: AgentId::from("executor")
                })
        );

        let candidates = registry
            .discover(DiscoveryQuery::new("rust").include_busy(true))
            .await
            .unwrap();
        assert!(candidates.is_empty());
        assert_eq!(
            settlement.get_hold(hold_id).await.unwrap().unwrap().status,
            HoldStatus::Refunded
        );
        assert_eq!(
            settlement.balance("publisher").await.unwrap(),
            100,
            "timeout refund returns escrowed funds to the payer"
        );
        assert_eq!(
            live_sessions
                .get_assignment(assignment_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            AssignmentStatus::Cancelled
        );
        assert!(
            tasks
                .active_tasks_by_agent("executor")
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            tasks.task_history_by_agent("executor").await.unwrap().len(),
            1,
            "runtime removes only the active participant entry"
        );

        registry.shutdown().await.unwrap();
        settlement.shutdown().await.unwrap();
        live_sessions.shutdown().await.unwrap();
        tasks.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn timeout_event_does_not_cancel_submitted_assignment_output() {
        let registry = RegistryService::spawn();
        let settlement = SettlementService::spawn();
        let live_sessions = LiveSessionService::spawn();
        let tasks = TaskService::spawn();
        let runtime = Runtime::new(
            registry.clone(),
            settlement.clone(),
            live_sessions.clone(),
            tasks.clone(),
        );

        let task_id = tasks.create("publisher", Timestamp(1)).await.unwrap();
        let session_id = live_sessions
            .create_session(task_id.clone(), Timestamp(2))
            .await
            .unwrap();
        let assignment_id = live_sessions
            .assign(
                task_id,
                session_id,
                "executor",
                AssignmentKind::Execute,
                Timestamp(3),
            )
            .await
            .unwrap();
        live_sessions
            .submit_output(
                assignment_id.clone(),
                "executor",
                "output-hash",
                Timestamp(4),
            )
            .await
            .unwrap();

        let report = runtime
            .handle_heartbeat_event_at(
                HeartbeatEvent::AgentTimedOut {
                    agent_id: AgentId::from("executor"),
                },
                Timestamp(5),
            )
            .await;

        assert!(
            !report
                .actions
                .contains(&RuntimeAction::AssignmentCancelled {
                    assignment_id: assignment_id.clone()
                })
        );
        assert_eq!(
            live_sessions
                .get_assignment(assignment_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            AssignmentStatus::Submitted
        );

        registry.shutdown().await.unwrap();
        settlement.shutdown().await.unwrap();
        live_sessions.shutdown().await.unwrap();
        tasks.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn timeout_event_refunds_only_holds_bound_to_timed_out_agent() {
        let registry = RegistryService::spawn();
        let settlement = SettlementService::spawn();
        let live_sessions = LiveSessionService::spawn();
        let tasks = TaskService::spawn();
        let runtime = Runtime::new(
            registry.clone(),
            settlement.clone(),
            live_sessions.clone(),
            tasks.clone(),
        );

        let task_id = tasks.create("publisher", Timestamp(1)).await.unwrap();
        let session_id = live_sessions
            .create_session(task_id.clone(), Timestamp(2))
            .await
            .unwrap();
        let assignment_id = live_sessions
            .assign(
                task_id.clone(),
                session_id,
                "executor",
                AssignmentKind::Execute,
                Timestamp(3),
            )
            .await
            .unwrap();
        settlement
            .deposit("publisher", 100, Timestamp(4))
            .await
            .unwrap();
        let hold_id = settlement
            .hold(
                "publisher",
                100,
                task_id,
                assignment_id,
                "executor",
                Timestamp(5),
            )
            .await
            .unwrap();

        let report = runtime
            .handle_heartbeat_event_at(
                HeartbeatEvent::AgentTimedOut {
                    agent_id: AgentId::from("publisher"),
                },
                Timestamp(6),
            )
            .await;

        assert!(!report.actions.contains(&RuntimeAction::HoldRefunded {
            hold_id: hold_id.clone()
        }));
        assert_eq!(
            settlement.get_hold(hold_id).await.unwrap().unwrap().status,
            HoldStatus::Active
        );
        assert_eq!(
            settlement.balance("publisher").await.unwrap(),
            0,
            "payer-side escrow should not be refunded just because the payer timed out"
        );

        registry.shutdown().await.unwrap();
        settlement.shutdown().await.unwrap();
        live_sessions.shutdown().await.unwrap();
        tasks.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn recovered_event_marks_agent_discoverable_again() {
        let registry = RegistryService::spawn();
        let settlement = SettlementService::spawn();
        let live_sessions = LiveSessionService::spawn();
        let tasks = TaskService::spawn();
        let runtime = Runtime::new(
            registry.clone(),
            settlement.clone(),
            live_sessions.clone(),
            tasks.clone(),
        );

        registry
            .register(AgentIdentity::new(AgentId::from("executor")))
            .await
            .unwrap();
        registry
            .declare_capabilities("executor", vec![Capability::new("rust", 1)])
            .await
            .unwrap();

        let report = runtime
            .handle_heartbeat_event_at(
                HeartbeatEvent::AgentRecovered {
                    agent_id: AgentId::from("executor"),
                },
                Timestamp(10),
            )
            .await;

        assert_eq!(
            report.actions,
            vec![RuntimeAction::RegistryMarkedAlive {
                agent_id: AgentId::from("executor")
            }]
        );
        let candidates = registry
            .discover(DiscoveryQuery::new("rust"))
            .await
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].agent_id, AgentId::from("executor"));

        registry.shutdown().await.unwrap();
        settlement.shutdown().await.unwrap();
        live_sessions.shutdown().await.unwrap();
        tasks.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn heartbeat_sink_forwards_timeout_events_to_runtime() {
        let registry = RegistryService::spawn();
        let settlement = SettlementService::spawn();
        let live_sessions = LiveSessionService::spawn();
        let tasks = TaskService::spawn();
        let runtime = Runtime::new(
            registry.clone(),
            settlement.clone(),
            live_sessions.clone(),
            tasks.clone(),
        );

        registry
            .register(AgentIdentity::new(AgentId::from("executor")))
            .await
            .unwrap();
        registry
            .declare_capabilities("executor", vec![Capability::new("rust", 1)])
            .await
            .unwrap();
        registry.mark_alive("executor").await.unwrap();

        let heartbeat = HeartbeatService::spawn(
            HeartbeatConfig {
                scan_interval: Duration::from_millis(5),
                idle_timeout: Duration::from_millis(20),
                busy_timeout: Duration::from_millis(20),
            },
            runtime.heartbeat_sink_with_clock(FixedRuntimeClock::new(Timestamp(100))),
        )
        .unwrap();

        heartbeat.ping("executor", false).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        let candidates = registry
            .discover(DiscoveryQuery::new("rust").include_busy(true))
            .await
            .unwrap();
        assert!(candidates.is_empty());

        heartbeat.shutdown().await.unwrap();
        registry.shutdown().await.unwrap();
        settlement.shutdown().await.unwrap();
        live_sessions.shutdown().await.unwrap();
        tasks.shutdown().await.unwrap();
    }
}
