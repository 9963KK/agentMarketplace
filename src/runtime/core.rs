use std::future::Future;

use crate::heartbeat::{AgentId, HeartbeatEvent, HeartbeatEventSink, PublishError};
use crate::livesession::{AssignmentStatus, LiveSessionHandle};
use crate::registry::RegistryHandle;
use crate::settlement::SettlementHandle;
use crate::task::TaskHandle;
use crate::types::Timestamp;

use super::clock::{RuntimeClock, SystemRuntimeClock};
use super::types::{RuntimeAction, RuntimeActionKind, RuntimeEventReport};

#[derive(Clone, Debug)]
pub struct Runtime {
    registry: RegistryHandle,
    settlement: SettlementHandle,
    live_sessions: LiveSessionHandle,
    tasks: TaskHandle,
}

impl Runtime {
    pub fn new(
        registry: RegistryHandle,
        settlement: SettlementHandle,
        live_sessions: LiveSessionHandle,
        tasks: TaskHandle,
    ) -> Self {
        Self {
            registry,
            settlement,
            live_sessions,
            tasks,
        }
    }

    pub fn heartbeat_sink(&self) -> RuntimeHeartbeatSink<SystemRuntimeClock> {
        RuntimeHeartbeatSink::new(self.clone(), SystemRuntimeClock)
    }

    pub fn heartbeat_sink_with_clock<C>(&self, clock: C) -> RuntimeHeartbeatSink<C>
    where
        C: RuntimeClock,
    {
        RuntimeHeartbeatSink::new(self.clone(), clock)
    }

    pub async fn handle_heartbeat_event_at(
        &self,
        event: HeartbeatEvent,
        at: Timestamp,
    ) -> RuntimeEventReport {
        let mut report = RuntimeEventReport::new(event.clone(), at);

        match event {
            HeartbeatEvent::AgentTimedOut { agent_id } => {
                self.handle_agent_timed_out(agent_id, at, &mut report).await;
            }
            HeartbeatEvent::AgentRecovered { agent_id } => {
                self.handle_agent_recovered(agent_id, &mut report).await;
            }
        }

        report
    }

    async fn handle_agent_timed_out(
        &self,
        agent_id: AgentId,
        at: Timestamp,
        report: &mut RuntimeEventReport,
    ) {
        if let Err(error) = self.registry.mark_timed_out(agent_id.clone()).await {
            report.record_error(
                RuntimeActionKind::MarkRegistryTimedOut,
                agent_id.to_string(),
                error,
            );
        } else {
            report.record_action(RuntimeAction::RegistryMarkedTimedOut {
                agent_id: agent_id.clone(),
            });
        }

        match self
            .settlement
            .active_holds_for_agent(agent_id.clone())
            .await
        {
            Ok(holds) => {
                for hold in holds {
                    if hold.agent_id != agent_id {
                        continue;
                    }

                    let hold_id = hold.hold_id;
                    if let Err(error) = self.settlement.refund(hold_id.clone(), at).await {
                        report.record_error(
                            RuntimeActionKind::RefundHold,
                            hold_id.to_string(),
                            error,
                        );
                    } else {
                        report.record_action(RuntimeAction::HoldRefunded { hold_id });
                    }
                }
            }
            Err(error) => report.record_error(
                RuntimeActionKind::ListActiveHoldsForAgent,
                agent_id.to_string(),
                error,
            ),
        }

        match self
            .live_sessions
            .assignments_by_agent(agent_id.clone())
            .await
        {
            Ok(assignments) => {
                for assignment in assignments {
                    if assignment.status != AssignmentStatus::Assigned {
                        continue;
                    }

                    let assignment_id = assignment.assignment_id;
                    if let Err(error) = self
                        .live_sessions
                        .cancel_assignment(assignment_id.clone(), at)
                        .await
                    {
                        report.record_error(
                            RuntimeActionKind::CancelAssignment,
                            assignment_id.to_string(),
                            error,
                        );
                    } else {
                        report.record_action(RuntimeAction::AssignmentCancelled { assignment_id });
                    }
                }
            }
            Err(error) => report.record_error(
                RuntimeActionKind::ListAssignmentsByAgent,
                agent_id.to_string(),
                error,
            ),
        }

        match self.tasks.active_tasks_by_agent(agent_id.clone()).await {
            Ok(tasks) => {
                for task in tasks {
                    let task_id = task.task_id;
                    match self
                        .tasks
                        .remove_participant(task_id.clone(), agent_id.clone(), at)
                        .await
                    {
                        Ok(true) => report.record_action(RuntimeAction::TaskParticipantRemoved {
                            task_id,
                            agent_id: agent_id.clone(),
                        }),
                        Ok(false) => {}
                        Err(error) => report.record_error(
                            RuntimeActionKind::RemoveTaskParticipant,
                            format!("{task_id}:{agent_id}"),
                            error,
                        ),
                    }
                }
            }
            Err(error) => report.record_error(
                RuntimeActionKind::ListActiveTasksByAgent,
                agent_id.to_string(),
                error,
            ),
        }
    }

    async fn handle_agent_recovered(&self, agent_id: AgentId, report: &mut RuntimeEventReport) {
        if let Err(error) = self.registry.mark_alive(agent_id.clone()).await {
            report.record_error(
                RuntimeActionKind::MarkRegistryAlive,
                agent_id.to_string(),
                error,
            );
        } else {
            report.record_action(RuntimeAction::RegistryMarkedAlive { agent_id });
        }
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeHeartbeatSink<C = SystemRuntimeClock> {
    runtime: Runtime,
    clock: C,
}

impl<C> RuntimeHeartbeatSink<C>
where
    C: RuntimeClock,
{
    pub fn new(runtime: Runtime, clock: C) -> Self {
        Self { runtime, clock }
    }
}

impl<C> HeartbeatEventSink for RuntimeHeartbeatSink<C>
where
    C: RuntimeClock,
{
    fn publish(
        &self,
        event: HeartbeatEvent,
    ) -> impl Future<Output = Result<(), PublishError>> + Send {
        let runtime = self.runtime.clone();
        let clock = self.clock.clone();

        async move {
            let report = runtime.handle_heartbeat_event_at(event, clock.now()).await;
            if report.has_errors() {
                eprintln!("runtime heartbeat cleanup had errors: {:?}", report.errors);
            }

            Ok(())
        }
    }
}
