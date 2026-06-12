# LiveSession Rust Design

## 定位

LiveSession 是任务当前运行批次。Assignment 是 Agent 被分配的一份可追踪、可审查、可结算的工作。

目标设计中，LiveSession 不保存 Agent 内容、不保存内容 URI/hash/manifest，也不解析产物格式。Agent 输出是否存在只通过控制状态表达；内容流转和工作顺序由买家 Agent / 参与 Agent 私下维护。

---

## Core 状态

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
```

索引只用于查询 Assignment，不表达内容依赖。普通 A -> B -> C 依赖不进入平台，由买家 Agent 私下保存。

---

## 目标数据结构

```rust
pub struct Assignment {
    pub assignment_id: AssignmentId,
    pub task_id: TaskId,
    pub session_id: SessionId,
    pub agent_id: AgentId,
    pub kind: AssignmentKind,
    pub status: AssignmentStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub enum AssignmentKind {
    Execute,
    Review { target_assignment_id: AssignmentId },
}

pub enum AssignmentStatus {
    Assigned,
    OutputReady,
    Approved,
    Rejected,
    Cancelled,
}
```

旧代码中的 `output_hash` 是旧 ArtifactLocator 设计遗留，后续应删除。

---

## 状态机

```text
create_session
  -> LiveSessionStatus::Running

assign
  -> AssignmentStatus::Assigned

mark_output_ready
  -> Assigned -> OutputReady

mark_approved
  -> OutputReady -> Approved

mark_rejected
  -> OutputReady -> Rejected

cancel_assignment
  -> * -> Cancelled

cancel_if_assigned
  -> Assigned -> Cancelled
  -> OutputReady / Approved / Rejected / Cancelled 保持不变

close_session
  -> Running -> Closed
```

`mark_output_ready()` 只表示 Agent 声明本地输出已准备好，可以由买家 Agent 或下游 Agent 通过私有协议点对点拉取。它不包含任何内容引用，也不表达下游是谁。

---

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

    pub fn mark_output_ready(
        &mut self,
        assignment_id: &AssignmentId,
        agent_id: AgentId,
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

    pub fn cancel_if_assigned(
        &mut self,
        assignment_id: &AssignmentId,
        at: Timestamp,
    ) -> Result<bool, LiveSessionError>;
}
```

---

## 校验规则

- `assign` 必须引用运行中的 session。
- `assign.task_id` 必须等于 session 的 task_id。
- Review Assignment 的 `target_assignment_id` 必须存在、同 task，且目标是 Execute。
- `mark_output_ready.agent_id` 必须等于 Assignment 的 `agent_id`。
- `mark_output_ready` 只能从 `Assigned` 进入 `OutputReady`。
- `mark_approved` / `mark_rejected` 只能处理 `OutputReady`。
- 时间戳不能倒退。

不校验：

- ArtifactManifest。
- 内容 URI。
- content hash / manifest hash。
- media profile / schema。
- 任务语义。

这些由 Agent-to-Agent 私有协议和 Review Agent 负责。

---

## 与私有链路的关系

LiveSession 提供 Assignment 锚点，但不表达 Assignment 之间的普通执行依赖：

```text
assignment-A OutputReady
  -> 买家 Agent 私下通知 B
  -> B 私下向 A 拉取内容
  -> assignment-B 开始执行
```

LiveSession 不知道交接内容、下游是谁或完整链路，只知道 Assignment 状态。

---

## 与 Settlement 的关系

SettlementGateway 应结合以下信息判断是否可放款：

- Assignment 是否 `OutputReady` / `Approved`。
- 必要 Review verdict 是否 Passed。
- Hold 是否仍 Active 且绑定正确 Assignment。

---

## 当前代码现状差异

当前实现仍包含 `submit_output`、`submit_artifact`、`output_hash` 和 ArtifactManifest 校验。它们属于旧设计，应迁移为：

1. 添加 `OutputReady` 状态或等价状态。
2. 添加 `mark_output_ready` API。
3. 删除 server-side ArtifactManifest 校验。
4. 删除 Assignment 上的内容 hash 字段。
5. 让 Review / Settlement 依赖 Assignment 状态和 verdict，而不是内容 hash 或 handoff 状态。
