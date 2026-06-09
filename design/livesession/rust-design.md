# LiveSession Rust Design

## 目标

LiveSession / Assignment 负责记录任务中的当前运行批次和工作单元。

它回答四个问题：

- 某个任务当前有哪些运行批次
- 某个批次下有哪些 Assignment
- 某个市场 Agent 承担了哪些 Assignment
- 某个 Assignment 是否已提交、通过、拒绝或取消

LiveSession 不保存链路顺序，不保存 artifact 内容，不做 Agent 选择，不做结算。它只给 Review / Settlement 提供稳定的 `assignment_id` 锚点，并用 `output_hash` 锚定 ArtifactManifest。

## 设计选择

采用和 Heartbeat / Registry / Task / Review / Settlement 一致的结构：

```text
LiveSessionCore      纯状态机
LiveSessionService   Tokio 命令循环
```

`LiveSessionCore` 不读系统时间，不访问网络，不调用 Task / Review / Settlement。所有时间由调用方传入，保证测试可控。

## 模块结构

```text
src/
├── livesession/
│   ├── mod.rs
│   ├── core.rs       # 纯状态机
│   ├── service.rs    # Tokio 命令循环
│   └── types.rs      # LiveSession、Assignment、状态、错误
├── types.rs          # SessionId、AssignmentId、TaskId、OutputHash、Timestamp
├── task/
├── review/
└── settlement/
```

## 核心数据模型

```rust
pub struct LiveSessionCore {
    sessions: HashMap<SessionId, LiveSession>,
    assignments: HashMap<AssignmentId, Assignment>,
    assignments_by_task: HashMap<TaskId, HashSet<AssignmentId>>,
    assignments_by_session: HashMap<SessionId, HashSet<AssignmentId>>,
    assignments_by_agent: HashMap<AgentId, HashSet<AssignmentId>>,
    review_assignments_by_target: HashMap<AssignmentId, HashSet<AssignmentId>>,
    next_session: u64,
    next_assignment: u64,
}

pub struct LiveSession {
    pub session_id: SessionId,
    pub task_id: TaskId,
    pub assignment_ids: HashSet<AssignmentId>,
    pub status: LiveSessionStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub struct Assignment {
    pub assignment_id: AssignmentId,
    pub task_id: TaskId,
    pub session_id: SessionId,
    pub agent_id: AgentId,
    pub kind: AssignmentKind,
    pub status: AssignmentStatus,
    pub output_hash: Option<OutputHash>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}
```

`output_hash` 当前沿用公共类型名，语义是 `artifact_manifest_hash`。完整 ArtifactManifest 遵守 `design/artifact/overview.md`，由 Agent 之间传递或由生产 Agent 保存，平台只记录 hash。

`sessions` 和 `assignments` 是真相源。索引用于快速查询：

- `assignments_by_task`：任务维度的 Assignment 列表
- `assignments_by_session`：运行批次维度的 Assignment 列表
- `assignments_by_agent`：Agent 维度的 Assignment 列表
- `review_assignments_by_target`：某个 Execute Assignment 绑定的 Review Assignment 列表

## Assignment 类型

```rust
pub enum AssignmentKind {
    Execute,
    Review { target_assignment_id: AssignmentId },
}
```

`Execute` 表示执行工作。`Review` 表示审查工作，且必须指向同一任务内的 Execute Assignment。

创建 Review Assignment 时，Core 校验：

- `target_assignment_id` 必须存在
- target 必须是 `AssignmentKind::Execute`
- target 的 `task_id` 必须和当前 assignment 的 `task_id` 一致

这样 reviewer 也是市场 Agent，也有自己的 Assignment、output 和结算锚点。

## 状态模型

```rust
pub enum LiveSessionStatus {
    Running,
    Closed,
}

pub enum AssignmentStatus {
    Assigned,
    Submitted,
    Approved,
    Rejected,
    Cancelled,
}
```

核心转移：

```text
create_session
  -> LiveSessionStatus::Running

assign
  -> AssignmentStatus::Assigned

submit_artifact / submit_output
  -> Assigned -> Submitted

mark_approved
  -> Submitted -> Approved

mark_rejected
  -> Submitted -> Rejected

cancel_assignment
  -> * -> Cancelled

cancel_if_assigned
  -> Assigned -> Cancelled
  -> Submitted / Approved / Rejected / Cancelled 保持不变

close_session
  -> Running -> Closed
```

`submit_artifact()` 是 Agent-facing 完成入口，必须由承担该 Assignment 的 Agent 调用。它校验 ArtifactManifest 的 task、assignment、producer、manifest hash 和 media profile，然后只保存 manifest hash。

`submit_output()` 是底层 raw hash 入口，语义同样是写入 `artifact_manifest_hash`。它保留给测试、迁移或调用方已经完成 Artifact 协议校验的场景。

`mark_approved()` 和 `mark_rejected()` 只接受 `Submitted` 状态。

`cancel_assignment()` 是底层状态操作，只校验 Assignment 存在和时间戳不倒退。业务调用通常应优先使用 `cancel_if_assigned()`。

`cancel_if_assigned()` 在 LiveSession 内部重新读取 Assignment 状态，只在状态仍是 `Assigned` 时取消；如果已经 `Submitted` 或进入终态，返回 `Ok(false)`。Runtime 掉线清理使用该接口，避免快照竞态覆盖已经提交的输出。

## 核心原语

```rust
impl LiveSessionCore {
    pub fn create_session(&mut self, task_id: TaskId, at: Timestamp) -> SessionId;

    pub fn close_session(
        &mut self,
        session_id: &SessionId,
        at: Timestamp,
    ) -> Result<(), LiveSessionError>;

    pub fn assign(
        &mut self,
        task_id: TaskId,
        session_id: &SessionId,
        agent_id: AgentId,
        kind: AssignmentKind,
        at: Timestamp,
    ) -> Result<AssignmentId, LiveSessionError>;

    pub fn submit_output(
        &mut self,
        assignment_id: &AssignmentId,
        agent_id: AgentId,
        output_hash: OutputHash, // artifact_manifest_hash
        at: Timestamp,
    ) -> Result<(), LiveSessionError>;

    pub fn submit_artifact(
        &mut self,
        assignment_id: &AssignmentId,
        agent_id: AgentId,
        manifest: ArtifactManifest,
        at: Timestamp,
    ) -> Result<(), LiveSessionError>;

    pub fn mark_approved(
        &mut self,
        assignment_id: &AssignmentId,
        at: Timestamp,
    ) -> Result<(), LiveSessionError>;

    pub fn mark_rejected(
        &mut self,
        assignment_id: &AssignmentId,
        at: Timestamp,
    ) -> Result<(), LiveSessionError>;

    pub fn cancel_assignment(
        &mut self,
        assignment_id: &AssignmentId,
        at: Timestamp,
    ) -> Result<(), LiveSessionError>;

    pub fn cancel_if_assigned(
        &mut self,
        assignment_id: &AssignmentId,
        at: Timestamp,
    ) -> Result<bool, LiveSessionError>;

    pub fn get_session(&self, session_id: &SessionId) -> Option<&LiveSession>;
    pub fn get_assignment(&self, assignment_id: &AssignmentId) -> Option<&Assignment>;
    pub fn assignments_by_task(&self, task_id: &TaskId) -> Vec<Assignment>;
    pub fn assignments_by_session(&self, session_id: &SessionId) -> Vec<Assignment>;
    pub fn assignments_by_agent(&self, agent_id: &AgentId) -> Vec<Assignment>;
    pub fn review_assignments_for_target(&self, target_assignment_id: &AssignmentId) -> Vec<Assignment>;
}
```

查询方法返回克隆快照，按 `AssignmentId` 升序排序，保证测试和调用方看到确定性顺序。

## 不变量

- `SessionId` 唯一
- `AssignmentId` 唯一
- Assignment 创建后绑定的 `task_id`、`session_id`、`agent_id` 不变
- 只有 `Running` session 可以新增 Assignment
- `assign.task_id` 必须等于 session 的 `task_id`
- Review Assignment 只能指向同任务内的 Execute Assignment
- 创建 Review Assignment 时必须写入 `review_assignments_by_target`
- `submit_artifact.agent_id` / `submit_output.agent_id` 必须等于 Assignment 的 `agent_id`
- `submit_artifact` / `submit_output` 只能发生在 `Assigned`
- `submit_artifact` 必须提交符合 Artifact Protocol 的 manifest，且 manifest 的 `task_id` / `assignment_id` / `producer_agent_id` 必须与 Assignment 匹配
- `cancel_if_assigned` 必须在同一个 Core 写操作内重新校验状态，不能依赖外部快照
- `mark_approved` / `mark_rejected` 只能发生在 `Submitted`
- 所有写操作时间戳不能小于目标对象当前 `updated_at`
- `assignments_by_*` 索引必须和 `assignments` 真相源保持一致

## 与其他组件的关系

LiveSession 不主动调用其他组件。调用方通常按这个顺序协作：

```text
Task.create()
Task.add_participant(task_id, agent_id)
LiveSession.create_session(task_id)
LiveSession.assign(task_id, session_id, agent_id, kind)
Settlement.hold(HoldRequest { task_id, assignment_id, agent_id, kind, ... })
Review.request(..., target_assignment_id, review_assignment_ids, ...)
```

Review 用 `target_assignment_id` 和 `review_assignment_id` 记录 verdict。
SettlementGateway 用 `review_assignments_for_target()` 确认 Execute Assignment 已挂载 Review Assignment，再结合 Review verdict 判断是否允许执行款 release。

Settlement 用 `assignment_id` 和 `agent_id` 绑定托管资金。

Artifact Protocol 用 `assignment_id` 绑定产物 manifest，LiveSession 只保存 manifest hash，不保存文件内容或 URL。

Runtime 在 Agent 掉线时查询 `assignments_by_agent(agent_id)`，但通过 `cancel_if_assigned()` 只取消仍处于 `Assigned` 状态的 Assignment。

## 错误处理

```rust
pub enum LiveSessionError {
    SessionNotFound(SessionId),
    SessionNotRunning { session_id: SessionId, status: LiveSessionStatus },
    AssignmentNotFound(AssignmentId),
    AssignmentNotAssigned { assignment_id: AssignmentId, status: AssignmentStatus },
    AssignmentNotSubmitted { assignment_id: AssignmentId, status: AssignmentStatus },
    AgentMismatch { assignment_id: AssignmentId, expected: AgentId, actual: AgentId },
    InvalidArtifact(ArtifactError),
    TargetAssignmentNotFound(AssignmentId),
    TargetAssignmentTaskMismatch {
        target_assignment_id: AssignmentId,
        expected_task_id: TaskId,
        actual_task_id: TaskId,
    },
    TargetAssignmentKindMismatch {
        target_assignment_id: AssignmentId,
        kind: AssignmentKind,
    },
    SessionTaskMismatch {
        session_id: SessionId,
        expected_task_id: TaskId,
        actual_task_id: TaskId,
    },
    TimestampWentBackwards { current: Timestamp, attempted: Timestamp },
}
```

所有写操作先校验，再更新状态和索引，避免部分写入。

## 服务层

```rust
pub enum LiveSessionCommand {
    CreateSession { task_id: TaskId, at: Timestamp, reply: oneshot::Sender<SessionId> },
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
    SubmitArtifact {
        assignment_id: AssignmentId,
        agent_id: AgentId,
        manifest: ArtifactManifest,
        at: Timestamp,
        reply: oneshot::Sender<Result<(), LiveSessionError>>,
    },
    MarkApproved { assignment_id: AssignmentId, at: Timestamp, reply: oneshot::Sender<Result<(), LiveSessionError>> },
    MarkRejected { assignment_id: AssignmentId, at: Timestamp, reply: oneshot::Sender<Result<(), LiveSessionError>> },
    CancelAssignment { assignment_id: AssignmentId, at: Timestamp, reply: oneshot::Sender<Result<(), LiveSessionError>> },
    CancelIfAssigned { assignment_id: AssignmentId, at: Timestamp, reply: oneshot::Sender<Result<bool, LiveSessionError>> },
    GetSession { session_id: SessionId, reply: oneshot::Sender<Option<LiveSession>> },
    GetAssignment { assignment_id: AssignmentId, reply: oneshot::Sender<Option<Assignment>> },
    AssignmentsByTask { task_id: TaskId, reply: oneshot::Sender<Vec<Assignment>> },
    AssignmentsBySession { session_id: SessionId, reply: oneshot::Sender<Vec<Assignment>> },
    AssignmentsByAgent { agent_id: AgentId, reply: oneshot::Sender<Vec<Assignment>> },
    ReviewAssignmentsForTarget { target_assignment_id: AssignmentId, reply: oneshot::Sender<Vec<Assignment>> },
    Shutdown { reply: oneshot::Sender<()> },
}
```

服务层职责：

- 顺序处理命令，避免锁扩散
- 将 `LiveSessionError` 包装为 `LiveSessionServiceError`
- 返回克隆快照，不暴露内部可变引用
- `Shutdown` 时退出循环后再确认调用方
- 不消费 Heartbeat / Review / Settlement 事件

## 测试策略

- create session 并生成确定性 `session-*`
- assign Execute 并建立 session / task / agent 索引
- assign Review 时校验 target 存在
- assign Review 时拒绝非 Execute target
- assign Review 时拒绝跨任务 target
- assign Review 后可通过 target 查询到 Review Assignment
- submit output 校验承担 Agent
- submit artifact 校验 task / assignment / producer / media profile 并保存 manifest hash
- submit artifact / output 只能从 `Assigned` 进入 `Submitted`
- cancel_if_assigned 不取消 `Submitted` Assignment
- mark approved / rejected 只能处理 `Submitted`
- close session 后拒绝新增 Assignment
- cancel assignment 更新状态
- 时间戳倒退返回 `TimestampWentBackwards`
- service 透传 core 错误
- service shutdown 后继续调用返回 `Stopped`

## 暂不做

- 链路 DAG
- 上下游输入输出边
- artifact 内容存储
- 自动选择 Agent
- 自动创建 Review
- 自动放款
- 分布式持久化

第一版只把执行、审查、结算锚定到 `assignment_id`。
