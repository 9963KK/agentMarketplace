use agent_marketplace::artifact::{
    ArtifactFile, ArtifactKind, ArtifactManifest, HashDigest, seal_manifest,
};
use agent_marketplace::heartbeat::{AgentId, HeartbeatEvent};
use agent_marketplace::livesession::{
    AssignmentKind, AssignmentStatus, LiveSessionHandle, LiveSessionService,
};
use agent_marketplace::registry::{
    AgentIdentity, Capability, DiscoveryQuery, RegistryHandle, RegistryService,
};
use agent_marketplace::review::{
    ReviewCriteria, ReviewHandle, ReviewService, Verdict, VerdictKind,
};
use agent_marketplace::runtime::Runtime;
use agent_marketplace::settlement::{
    HoldStatus, ReleaseEvidence, SettlementHandle, SettlementService,
};
use agent_marketplace::task::{TaskHandle, TaskService, TaskStatus};
use agent_marketplace::types::{AssignmentId, TaskId, Timestamp};

struct ComponentStack {
    registry: RegistryHandle,
    tasks: TaskHandle,
    live_sessions: LiveSessionHandle,
    review: ReviewHandle,
    settlement: SettlementHandle,
}

impl ComponentStack {
    fn spawn() -> Self {
        Self {
            registry: RegistryService::spawn(),
            tasks: TaskService::spawn(),
            live_sessions: LiveSessionService::spawn(),
            review: ReviewService::spawn(),
            settlement: SettlementService::spawn(),
        }
    }

    fn runtime(&self) -> Runtime {
        Runtime::new(
            self.registry.clone(),
            self.settlement.clone(),
            self.live_sessions.clone(),
            self.tasks.clone(),
        )
    }

    async fn shutdown(self) {
        let _ = self.registry.shutdown().await;
        let _ = self.tasks.shutdown().await;
        let _ = self.live_sessions.shutdown().await;
        let _ = self.review.shutdown().await;
        let _ = self.settlement.shutdown().await;
    }
}

fn agent(id: &str) -> AgentId {
    AgentId::from(id)
}

fn hash(value: u8) -> HashDigest {
    HashDigest::from_sha256_hex(format!("{value:064x}")).unwrap()
}

fn text_manifest(
    artifact_id: &str,
    task_id: &TaskId,
    assignment_id: &AssignmentId,
    producer_agent_id: &AgentId,
    at: Timestamp,
) -> ArtifactManifest {
    let file = ArtifactFile::new(
        format!("https://agent.example/{artifact_id}.md"),
        hash(1),
        "text/markdown",
        "text.markdown.utf8.v1",
        120,
    );
    seal_manifest(ArtifactManifest::new(
        artifact_id,
        task_id.clone(),
        assignment_id.clone(),
        producer_agent_id.clone(),
        ArtifactKind::Single,
        vec![file],
        at,
    ))
    .unwrap()
}

async fn submit_text_artifact(
    live_sessions: &LiveSessionHandle,
    task_id: &TaskId,
    assignment_id: &AssignmentId,
    producer_agent_id: &AgentId,
    artifact_id: &str,
    at: Timestamp,
) {
    let manifest = text_manifest(artifact_id, task_id, assignment_id, producer_agent_id, at);
    live_sessions
        .submit_artifact(
            assignment_id.clone(),
            producer_agent_id.clone(),
            manifest,
            at,
        )
        .await
        .unwrap();
}

fn passed_verdict() -> Verdict {
    Verdict {
        kind: VerdictKind::Passed,
        score_bps: 10_000,
        feedback: "accepted".to_string(),
    }
}

fn failed_verdict() -> Verdict {
    Verdict {
        kind: VerdictKind::Failed,
        score_bps: 0,
        feedback: "rejected by review policy".to_string(),
    }
}

async fn register_agent(registry: &RegistryHandle, agent_id: AgentId, capability: &str) {
    registry
        .register(AgentIdentity::new(agent_id.clone()))
        .await
        .unwrap();
    registry
        .declare_capabilities(agent_id.clone(), vec![Capability::new(capability, 1)])
        .await
        .unwrap();
    registry.mark_alive(agent_id).await.unwrap();
}

#[tokio::test]
async fn happy_path_coordinates_task_assignment_review_and_settlement() {
    let stack = ComponentStack::spawn();
    let publisher = agent("publisher");
    let executor = agent("executor");
    let reviewer = agent("reviewer");

    register_agent(&stack.registry, executor.clone(), "execute").await;
    register_agent(&stack.registry, reviewer.clone(), "review").await;
    stack
        .settlement
        .deposit(publisher.clone(), 130, Timestamp(1))
        .await
        .unwrap();

    assert_eq!(
        stack
            .registry
            .discover(DiscoveryQuery::new("execute"))
            .await
            .unwrap()[0]
            .agent_id,
        executor
    );
    assert_eq!(
        stack
            .registry
            .discover(DiscoveryQuery::new("review"))
            .await
            .unwrap()[0]
            .agent_id,
        reviewer
    );

    let task_id = stack
        .tasks
        .create(publisher.clone(), Timestamp(2))
        .await
        .unwrap();
    let session_id = stack
        .live_sessions
        .create_session(task_id.clone(), Timestamp(3))
        .await
        .unwrap();

    stack
        .tasks
        .add_participant(task_id.clone(), executor.clone(), Timestamp(4))
        .await
        .unwrap();
    let execute_assignment = stack
        .live_sessions
        .assign(
            task_id.clone(),
            session_id.clone(),
            executor.clone(),
            AssignmentKind::Execute,
            Timestamp(5),
        )
        .await
        .unwrap();
    let execute_hold = stack
        .settlement
        .hold(
            publisher.clone(),
            100,
            task_id.clone(),
            execute_assignment.clone(),
            executor.clone(),
            Timestamp(6),
        )
        .await
        .unwrap();
    submit_text_artifact(
        &stack.live_sessions,
        &task_id,
        &execute_assignment,
        &executor,
        "execute-output",
        Timestamp(7),
    )
    .await;

    stack
        .tasks
        .add_participant(task_id.clone(), reviewer.clone(), Timestamp(8))
        .await
        .unwrap();
    let review_assignment = stack
        .live_sessions
        .assign(
            task_id.clone(),
            session_id,
            reviewer.clone(),
            AssignmentKind::Review {
                target_assignment_id: execute_assignment.clone(),
            },
            Timestamp(9),
        )
        .await
        .unwrap();
    let review_hold = stack
        .settlement
        .hold(
            publisher.clone(),
            30,
            task_id.clone(),
            review_assignment.clone(),
            reviewer.clone(),
            Timestamp(10),
        )
        .await
        .unwrap();
    let review_id = stack
        .review
        .request(
            task_id.clone(),
            execute_assignment.clone(),
            vec![review_assignment.clone()],
            ReviewCriteria::plain_text("review the submitted output"),
            Timestamp(11),
        )
        .await
        .unwrap();
    submit_text_artifact(
        &stack.live_sessions,
        &task_id,
        &review_assignment,
        &reviewer,
        "review-output",
        Timestamp(12),
    )
    .await;
    stack
        .review
        .submit(
            review_id.clone(),
            review_assignment.clone(),
            passed_verdict(),
            Timestamp(13),
        )
        .await
        .unwrap();
    stack
        .settlement
        .release(
            review_hold.clone(),
            ReleaseEvidence::ReviewSubmitted {
                task_id: task_id.clone(),
                assignment_id: review_assignment.clone(),
                review_id: review_id.clone(),
            },
            Timestamp(14),
        )
        .await
        .unwrap();
    stack
        .live_sessions
        .mark_approved(review_assignment.clone(), Timestamp(15))
        .await
        .unwrap();
    stack
        .settlement
        .release(
            execute_hold.clone(),
            ReleaseEvidence::AssignmentOutputAccepted {
                task_id: task_id.clone(),
                assignment_id: execute_assignment.clone(),
                review_ids: vec![review_id.clone()],
            },
            Timestamp(16),
        )
        .await
        .unwrap();
    stack
        .live_sessions
        .mark_approved(execute_assignment.clone(), Timestamp(17))
        .await
        .unwrap();
    stack
        .tasks
        .complete(task_id.clone(), Timestamp(18))
        .await
        .unwrap();

    assert_eq!(stack.settlement.balance(publisher).await.unwrap(), 0);
    assert_eq!(stack.settlement.balance(executor).await.unwrap(), 100);
    assert_eq!(stack.settlement.balance(reviewer).await.unwrap(), 30);
    assert_eq!(
        stack
            .settlement
            .get_hold(execute_hold)
            .await
            .unwrap()
            .unwrap()
            .status,
        HoldStatus::Released
    );
    assert_eq!(
        stack
            .settlement
            .get_hold(review_hold)
            .await
            .unwrap()
            .unwrap()
            .status,
        HoldStatus::Released
    );
    assert_eq!(
        stack
            .live_sessions
            .get_assignment(execute_assignment.clone())
            .await
            .unwrap()
            .unwrap()
            .status,
        AssignmentStatus::Approved
    );
    assert_eq!(
        stack
            .live_sessions
            .get_assignment(review_assignment)
            .await
            .unwrap()
            .unwrap()
            .status,
        AssignmentStatus::Approved
    );
    assert_eq!(
        stack
            .review
            .collect_by_assignment(execute_assignment)
            .await
            .unwrap()[0]
            .verdicts
            .len(),
        1
    );
    assert_eq!(
        stack
            .tasks
            .get(task_id.clone())
            .await
            .unwrap()
            .unwrap()
            .status,
        TaskStatus::Completed
    );
    assert!(
        stack
            .tasks
            .active_tasks_by_agent("executor")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        stack
            .tasks
            .task_history_by_agent("executor")
            .await
            .unwrap()
            .len(),
        1
    );

    stack.shutdown().await;
}

#[tokio::test]
async fn timeout_before_submission_refunds_and_cancels_current_work() {
    let stack = ComponentStack::spawn();
    let runtime = stack.runtime();
    let publisher = agent("publisher");
    let executor = agent("executor");

    register_agent(&stack.registry, executor.clone(), "execute").await;
    stack
        .settlement
        .deposit(publisher.clone(), 100, Timestamp(1))
        .await
        .unwrap();
    let task_id = stack
        .tasks
        .create(publisher.clone(), Timestamp(2))
        .await
        .unwrap();
    let session_id = stack
        .live_sessions
        .create_session(task_id.clone(), Timestamp(3))
        .await
        .unwrap();
    stack
        .tasks
        .add_participant(task_id.clone(), executor.clone(), Timestamp(4))
        .await
        .unwrap();
    let assignment_id = stack
        .live_sessions
        .assign(
            task_id.clone(),
            session_id,
            executor.clone(),
            AssignmentKind::Execute,
            Timestamp(5),
        )
        .await
        .unwrap();
    let hold_id = stack
        .settlement
        .hold(
            publisher.clone(),
            100,
            task_id.clone(),
            assignment_id.clone(),
            executor.clone(),
            Timestamp(6),
        )
        .await
        .unwrap();

    let report = runtime
        .handle_heartbeat_event_at(
            HeartbeatEvent::AgentTimedOut {
                agent_id: executor.clone(),
            },
            Timestamp(7),
        )
        .await;

    assert!(!report.has_errors());
    assert_eq!(
        stack
            .settlement
            .get_hold(hold_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        HoldStatus::Refunded
    );
    assert_eq!(stack.settlement.balance(publisher).await.unwrap(), 100);
    assert_eq!(
        stack
            .live_sessions
            .get_assignment(assignment_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        AssignmentStatus::Cancelled
    );
    assert!(
        stack
            .registry
            .discover(DiscoveryQuery::new("execute").include_busy(true))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        stack
            .tasks
            .active_tasks_by_agent(executor.clone())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        stack
            .tasks
            .task_history_by_agent(executor)
            .await
            .unwrap()
            .len(),
        1
    );

    stack.shutdown().await;
}

#[tokio::test]
async fn timeout_after_submission_preserves_output_and_escrow() {
    let stack = ComponentStack::spawn();
    let runtime = stack.runtime();
    let publisher = agent("publisher");
    let executor = agent("executor");

    register_agent(&stack.registry, executor.clone(), "execute").await;
    stack
        .settlement
        .deposit(publisher.clone(), 100, Timestamp(1))
        .await
        .unwrap();
    let task_id = stack
        .tasks
        .create(publisher.clone(), Timestamp(2))
        .await
        .unwrap();
    let session_id = stack
        .live_sessions
        .create_session(task_id.clone(), Timestamp(3))
        .await
        .unwrap();
    stack
        .tasks
        .add_participant(task_id.clone(), executor.clone(), Timestamp(4))
        .await
        .unwrap();
    let assignment_id = stack
        .live_sessions
        .assign(
            task_id.clone(),
            session_id,
            executor.clone(),
            AssignmentKind::Execute,
            Timestamp(5),
        )
        .await
        .unwrap();
    let hold_id = stack
        .settlement
        .hold(
            publisher.clone(),
            100,
            task_id.clone(),
            assignment_id.clone(),
            executor.clone(),
            Timestamp(6),
        )
        .await
        .unwrap();
    submit_text_artifact(
        &stack.live_sessions,
        &task_id,
        &assignment_id,
        &executor,
        "execute-output-timeout-preserved",
        Timestamp(7),
    )
    .await;

    let report = runtime
        .handle_heartbeat_event_at(
            HeartbeatEvent::AgentTimedOut {
                agent_id: executor.clone(),
            },
            Timestamp(8),
        )
        .await;

    assert!(!report.has_errors());
    assert_eq!(
        stack
            .settlement
            .get_hold(hold_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        HoldStatus::Active,
        "submitted work keeps escrow for later review / release"
    );
    assert_eq!(stack.settlement.balance(publisher).await.unwrap(), 0);
    assert_eq!(
        stack
            .live_sessions
            .get_assignment(assignment_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        AssignmentStatus::Submitted
    );
    assert!(
        stack
            .tasks
            .active_tasks_by_agent(executor)
            .await
            .unwrap()
            .is_empty()
    );

    stack.shutdown().await;
}

#[tokio::test]
async fn reviewer_timeout_refunds_only_reviewer_hold() {
    let stack = ComponentStack::spawn();
    let runtime = stack.runtime();
    let publisher = agent("publisher");
    let executor = agent("executor");
    let reviewer = agent("reviewer");

    register_agent(&stack.registry, executor.clone(), "execute").await;
    register_agent(&stack.registry, reviewer.clone(), "review").await;
    stack
        .settlement
        .deposit(publisher.clone(), 130, Timestamp(1))
        .await
        .unwrap();
    let task_id = stack
        .tasks
        .create(publisher.clone(), Timestamp(2))
        .await
        .unwrap();
    let session_id = stack
        .live_sessions
        .create_session(task_id.clone(), Timestamp(3))
        .await
        .unwrap();
    stack
        .tasks
        .add_participant(task_id.clone(), executor.clone(), Timestamp(4))
        .await
        .unwrap();
    let execute_assignment = stack
        .live_sessions
        .assign(
            task_id.clone(),
            session_id.clone(),
            executor.clone(),
            AssignmentKind::Execute,
            Timestamp(5),
        )
        .await
        .unwrap();
    let execute_hold = stack
        .settlement
        .hold(
            publisher.clone(),
            100,
            task_id.clone(),
            execute_assignment.clone(),
            executor.clone(),
            Timestamp(6),
        )
        .await
        .unwrap();
    submit_text_artifact(
        &stack.live_sessions,
        &task_id,
        &execute_assignment,
        &executor,
        "execute-output-reviewer-timeout",
        Timestamp(7),
    )
    .await;

    stack
        .tasks
        .add_participant(task_id.clone(), reviewer.clone(), Timestamp(8))
        .await
        .unwrap();
    let review_assignment = stack
        .live_sessions
        .assign(
            task_id.clone(),
            session_id,
            reviewer.clone(),
            AssignmentKind::Review {
                target_assignment_id: execute_assignment.clone(),
            },
            Timestamp(9),
        )
        .await
        .unwrap();
    let review_hold = stack
        .settlement
        .hold(
            publisher.clone(),
            30,
            task_id.clone(),
            review_assignment.clone(),
            reviewer.clone(),
            Timestamp(10),
        )
        .await
        .unwrap();
    stack
        .review
        .request(
            task_id,
            execute_assignment.clone(),
            vec![review_assignment.clone()],
            ReviewCriteria::plain_text("review the submitted output"),
            Timestamp(11),
        )
        .await
        .unwrap();

    let report = runtime
        .handle_heartbeat_event_at(
            HeartbeatEvent::AgentTimedOut {
                agent_id: reviewer.clone(),
            },
            Timestamp(12),
        )
        .await;

    assert!(!report.has_errors());
    assert_eq!(
        stack
            .settlement
            .get_hold(review_hold)
            .await
            .unwrap()
            .unwrap()
            .status,
        HoldStatus::Refunded
    );
    assert_eq!(
        stack
            .settlement
            .get_hold(execute_hold)
            .await
            .unwrap()
            .unwrap()
            .status,
        HoldStatus::Active,
        "reviewer timeout must not refund executor escrow"
    );
    assert_eq!(
        stack
            .live_sessions
            .get_assignment(review_assignment)
            .await
            .unwrap()
            .unwrap()
            .status,
        AssignmentStatus::Cancelled
    );
    assert_eq!(
        stack
            .live_sessions
            .get_assignment(execute_assignment.clone())
            .await
            .unwrap()
            .unwrap()
            .status,
        AssignmentStatus::Submitted
    );
    assert_eq!(
        stack
            .review
            .collect_by_assignment(execute_assignment)
            .await
            .unwrap()[0]
            .verdicts
            .len(),
        0
    );
    assert_eq!(stack.settlement.balance(publisher).await.unwrap(), 30);
    assert!(
        stack
            .tasks
            .active_tasks_by_agent(reviewer)
            .await
            .unwrap()
            .is_empty()
    );

    stack.shutdown().await;
}

#[tokio::test]
async fn business_flow_replaces_timed_out_reviewer_and_completes_task() {
    let stack = ComponentStack::spawn();
    let runtime = stack.runtime();
    let publisher = agent("publisher");
    let executor = agent("executor");
    let reviewer_1 = agent("reviewer-1");
    let reviewer_2 = agent("reviewer-2");

    register_agent(&stack.registry, executor.clone(), "execute").await;
    register_agent(&stack.registry, reviewer_1.clone(), "review").await;
    register_agent(&stack.registry, reviewer_2.clone(), "review").await;
    stack
        .settlement
        .deposit(publisher.clone(), 130, Timestamp(1))
        .await
        .unwrap();

    let task_id = stack
        .tasks
        .create(publisher.clone(), Timestamp(2))
        .await
        .unwrap();
    let session_id = stack
        .live_sessions
        .create_session(task_id.clone(), Timestamp(3))
        .await
        .unwrap();
    stack
        .tasks
        .add_participant(task_id.clone(), executor.clone(), Timestamp(4))
        .await
        .unwrap();
    let execute_assignment = stack
        .live_sessions
        .assign(
            task_id.clone(),
            session_id.clone(),
            executor.clone(),
            AssignmentKind::Execute,
            Timestamp(5),
        )
        .await
        .unwrap();
    let execute_hold = stack
        .settlement
        .hold(
            publisher.clone(),
            100,
            task_id.clone(),
            execute_assignment.clone(),
            executor.clone(),
            Timestamp(6),
        )
        .await
        .unwrap();
    submit_text_artifact(
        &stack.live_sessions,
        &task_id,
        &execute_assignment,
        &executor,
        "execute-output-replacement-review",
        Timestamp(7),
    )
    .await;

    stack
        .tasks
        .add_participant(task_id.clone(), reviewer_1.clone(), Timestamp(8))
        .await
        .unwrap();
    let first_review_assignment = stack
        .live_sessions
        .assign(
            task_id.clone(),
            session_id.clone(),
            reviewer_1.clone(),
            AssignmentKind::Review {
                target_assignment_id: execute_assignment.clone(),
            },
            Timestamp(9),
        )
        .await
        .unwrap();
    let first_review_hold = stack
        .settlement
        .hold(
            publisher.clone(),
            30,
            task_id.clone(),
            first_review_assignment.clone(),
            reviewer_1.clone(),
            Timestamp(10),
        )
        .await
        .unwrap();
    let first_review_id = stack
        .review
        .request(
            task_id.clone(),
            execute_assignment.clone(),
            vec![first_review_assignment.clone()],
            ReviewCriteria::plain_text("first review attempt"),
            Timestamp(11),
        )
        .await
        .unwrap();

    runtime
        .handle_heartbeat_event_at(
            HeartbeatEvent::AgentTimedOut {
                agent_id: reviewer_1.clone(),
            },
            Timestamp(12),
        )
        .await;

    assert_eq!(
        stack
            .settlement
            .get_hold(first_review_hold)
            .await
            .unwrap()
            .unwrap()
            .status,
        HoldStatus::Refunded
    );
    assert_eq!(
        stack
            .live_sessions
            .get_assignment(first_review_assignment)
            .await
            .unwrap()
            .unwrap()
            .status,
        AssignmentStatus::Cancelled
    );

    stack
        .tasks
        .add_participant(task_id.clone(), reviewer_2.clone(), Timestamp(13))
        .await
        .unwrap();
    let second_review_assignment = stack
        .live_sessions
        .assign(
            task_id.clone(),
            session_id,
            reviewer_2.clone(),
            AssignmentKind::Review {
                target_assignment_id: execute_assignment.clone(),
            },
            Timestamp(14),
        )
        .await
        .unwrap();
    let second_review_hold = stack
        .settlement
        .hold(
            publisher.clone(),
            30,
            task_id.clone(),
            second_review_assignment.clone(),
            reviewer_2.clone(),
            Timestamp(15),
        )
        .await
        .unwrap();
    let second_review_id = stack
        .review
        .request(
            task_id.clone(),
            execute_assignment.clone(),
            vec![second_review_assignment.clone()],
            ReviewCriteria::plain_text("replacement review attempt"),
            Timestamp(16),
        )
        .await
        .unwrap();
    submit_text_artifact(
        &stack.live_sessions,
        &task_id,
        &second_review_assignment,
        &reviewer_2,
        "replacement-review-output",
        Timestamp(17),
    )
    .await;
    stack
        .review
        .submit(
            second_review_id.clone(),
            second_review_assignment.clone(),
            passed_verdict(),
            Timestamp(18),
        )
        .await
        .unwrap();
    stack
        .settlement
        .release(
            second_review_hold,
            ReleaseEvidence::ReviewSubmitted {
                task_id: task_id.clone(),
                assignment_id: second_review_assignment.clone(),
                review_id: second_review_id.clone(),
            },
            Timestamp(19),
        )
        .await
        .unwrap();
    stack
        .live_sessions
        .mark_approved(second_review_assignment, Timestamp(20))
        .await
        .unwrap();
    stack
        .settlement
        .release(
            execute_hold,
            ReleaseEvidence::AssignmentOutputAccepted {
                task_id: task_id.clone(),
                assignment_id: execute_assignment.clone(),
                review_ids: vec![second_review_id],
            },
            Timestamp(21),
        )
        .await
        .unwrap();
    stack
        .live_sessions
        .mark_approved(execute_assignment.clone(), Timestamp(22))
        .await
        .unwrap();
    stack
        .tasks
        .complete(task_id.clone(), Timestamp(23))
        .await
        .unwrap();

    let review_sessions = stack
        .review
        .collect_by_assignment(execute_assignment)
        .await
        .unwrap();
    assert_eq!(review_sessions.len(), 2);
    assert_eq!(
        stack
            .review
            .collect(first_review_id)
            .await
            .unwrap()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(stack.settlement.balance(publisher).await.unwrap(), 0);
    assert_eq!(stack.settlement.balance(executor).await.unwrap(), 100);
    assert_eq!(stack.settlement.balance(reviewer_1).await.unwrap(), 0);
    assert_eq!(stack.settlement.balance(reviewer_2).await.unwrap(), 30);
    assert_eq!(
        stack.tasks.get(task_id).await.unwrap().unwrap().status,
        TaskStatus::Completed
    );

    stack.shutdown().await;
}

#[tokio::test]
async fn business_flow_failed_review_pays_reviewer_refunds_executor_and_cancels_task() {
    let stack = ComponentStack::spawn();
    let publisher = agent("publisher");
    let executor = agent("executor");
    let reviewer = agent("reviewer");

    register_agent(&stack.registry, executor.clone(), "execute").await;
    register_agent(&stack.registry, reviewer.clone(), "review").await;
    stack
        .settlement
        .deposit(publisher.clone(), 130, Timestamp(1))
        .await
        .unwrap();

    let task_id = stack
        .tasks
        .create(publisher.clone(), Timestamp(2))
        .await
        .unwrap();
    let session_id = stack
        .live_sessions
        .create_session(task_id.clone(), Timestamp(3))
        .await
        .unwrap();
    stack
        .tasks
        .add_participant(task_id.clone(), executor.clone(), Timestamp(4))
        .await
        .unwrap();
    let execute_assignment = stack
        .live_sessions
        .assign(
            task_id.clone(),
            session_id.clone(),
            executor.clone(),
            AssignmentKind::Execute,
            Timestamp(5),
        )
        .await
        .unwrap();
    let execute_hold = stack
        .settlement
        .hold(
            publisher.clone(),
            100,
            task_id.clone(),
            execute_assignment.clone(),
            executor.clone(),
            Timestamp(6),
        )
        .await
        .unwrap();
    submit_text_artifact(
        &stack.live_sessions,
        &task_id,
        &execute_assignment,
        &executor,
        "execute-output-failed-review",
        Timestamp(7),
    )
    .await;

    stack
        .tasks
        .add_participant(task_id.clone(), reviewer.clone(), Timestamp(8))
        .await
        .unwrap();
    let review_assignment = stack
        .live_sessions
        .assign(
            task_id.clone(),
            session_id,
            reviewer.clone(),
            AssignmentKind::Review {
                target_assignment_id: execute_assignment.clone(),
            },
            Timestamp(9),
        )
        .await
        .unwrap();
    let review_hold = stack
        .settlement
        .hold(
            publisher.clone(),
            30,
            task_id.clone(),
            review_assignment.clone(),
            reviewer.clone(),
            Timestamp(10),
        )
        .await
        .unwrap();
    let review_id = stack
        .review
        .request(
            task_id.clone(),
            execute_assignment.clone(),
            vec![review_assignment.clone()],
            ReviewCriteria::plain_text("reject invalid output"),
            Timestamp(11),
        )
        .await
        .unwrap();
    submit_text_artifact(
        &stack.live_sessions,
        &task_id,
        &review_assignment,
        &reviewer,
        "review-output-failed-review",
        Timestamp(12),
    )
    .await;
    stack
        .review
        .submit(
            review_id.clone(),
            review_assignment.clone(),
            failed_verdict(),
            Timestamp(13),
        )
        .await
        .unwrap();

    stack
        .settlement
        .release(
            review_hold,
            ReleaseEvidence::ReviewSubmitted {
                task_id: task_id.clone(),
                assignment_id: review_assignment.clone(),
                review_id: review_id.clone(),
            },
            Timestamp(14),
        )
        .await
        .unwrap();
    stack
        .live_sessions
        .mark_approved(review_assignment, Timestamp(15))
        .await
        .unwrap();
    stack
        .settlement
        .refund(execute_hold, Timestamp(16))
        .await
        .unwrap();
    stack
        .live_sessions
        .mark_rejected(execute_assignment.clone(), Timestamp(17))
        .await
        .unwrap();
    stack
        .tasks
        .cancel(task_id.clone(), Timestamp(18))
        .await
        .unwrap();

    let verdicts = stack.review.collect(review_id).await.unwrap().unwrap();
    assert_eq!(verdicts[0].verdict.kind, VerdictKind::Failed);
    assert_eq!(stack.settlement.balance(publisher).await.unwrap(), 100);
    assert_eq!(stack.settlement.balance(executor).await.unwrap(), 0);
    assert_eq!(stack.settlement.balance(reviewer).await.unwrap(), 30);
    assert_eq!(
        stack
            .live_sessions
            .get_assignment(execute_assignment)
            .await
            .unwrap()
            .unwrap()
            .status,
        AssignmentStatus::Rejected
    );
    assert_eq!(
        stack.tasks.get(task_id).await.unwrap().unwrap().status,
        TaskStatus::Cancelled
    );

    stack.shutdown().await;
}
