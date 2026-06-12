# LiveSession / Assignment

## 定位

LiveSession 是任务当前运行批次。Assignment 是市场 Agent 被分配的一份可追踪、可审查、可结算的工作。

LiveSession 只保存 Assignment 控制状态，不保存链路内容、不保存输入输出边上传递的 payload、不保存 hash/URI/manifest，也不保存 Agent 工作顺序。上下游流转由买家 Agent 和参与 Agent 通过私有 handoff 协议维护。

---

## 核心原则

| 原则 | 说明 |
|------|------|
| 所有 Agent 平等 | executor、reviewer、planner、aggregator 都是市场 Agent |
| Assignment 是最小锚点 | 完成、审查、结算都绑定 assignment_id |
| Review Agent 不是附属字段 | reviewer 也有自己的 Assignment |
| 链路交接由 Agent 私下维护 | A -> B -> C 不放在 LiveSession 内，也不进入平台其他组件 |
| 不保存内容或内容元数据 | 不记录 manifest、URI、hash、schema、文件名 |

---

## 数据模型

```rust
struct LiveSession {
    session_id: SessionId,
    task_id: TaskId,
    assignment_ids: HashSet<AssignmentId>,
    status: LiveSessionStatus,
    created_at: Timestamp,
    updated_at: Timestamp,
}

struct Assignment {
    assignment_id: AssignmentId,
    task_id: TaskId,
    session_id: SessionId,
    agent_id: AgentId,
    kind: AssignmentKind,
    status: AssignmentStatus,
    created_at: Timestamp,
    updated_at: Timestamp,
}

enum AssignmentKind {
    Execute,
    Review { target_assignment_id: AssignmentId },
}

enum AssignmentStatus {
    Assigned,
    OutputReady,
    Approved,
    Rejected,
    Cancelled,
}
```

`Review { target_assignment_id }` 只表达审查关系，不表达内容位置。

---

## 原语

| 原语 | 说明 |
|------|------|
| `create_session(task_id, at)` | 创建当前运行批次 |
| `close_session(session_id, at)` | 关闭运行批次 |
| `assign(task_id, session_id, agent_id, kind, at)` | 创建 Assignment |
| `mark_output_ready(assignment_id, agent_id, at)` | Agent 声明本地输出已准备好，可通过私有协议交给下游或 reviewer |
| `mark_approved(assignment_id, at)` | 标记 Assignment 审查通过 |
| `mark_rejected(assignment_id, at)` | 标记 Assignment 审查失败 |
| `cancel_assignment(assignment_id, at)` | 取消 Assignment |
| `cancel_if_assigned(assignment_id, at)` | 仅当 Assignment 仍未产出时取消 |
| `assignments_by_agent(agent_id)` | 查询某 Agent 的 Assignment |
| `review_assignments_for_target(target_assignment_id)` | 查询某个 Execute Assignment 绑定的 Review Assignment |

---

## 完成语义

平台不能凭空知道 Agent 完成了工作。完成必须由承担该 Assignment 的 Agent 上报：

```text
mark_output_ready(assignment_id, agent_id)
```

平台只校验：

- `assignment_id` 存在。
- `agent_id == assignment.agent_id`。
- Assignment 仍处于可完成状态。
- 时间戳不倒退。

平台不要求上传 ArtifactManifest，不校验 hash，不读取 URI，不解析 media profile。真实内容通过 Agent 私有协议点对点交给下游或 Review Agent。

---

## Review 关系

Review 绑定 Assignment，而不是只绑定 Task。

```text
Review Assignment:
  assignment_id = review_assignment_id
  kind = Review { target_assignment_id = execute_assignment_id }

Review verdict:
  review_assignment_id
  target_assignment_id
  verdict
```

Review Agent 私下拉取目标内容并校验格式/语义，然后只把 verdict 提交给平台。

---

## Settlement 关系

Settlement 按 Assignment 和 Review 状态结算：

- Execute hold 释放依赖目标 Assignment 的完成状态和 Review verdict。
- Review hold 释放依赖 Review Assignment 和 verdict 提交。
- 私有 handoff 失败由买家 Agent 或 Review Agent 表达为重排、取消、退款请求或 review verdict。

---

## 当前代码现状差异

当前代码仍有 `output_hash`、`submit_output`、`submit_artifact` 和 ArtifactManifest 校验，是旧设计。后续应删除平台内容 hash 字段和 manifest 校验入口，改为 `mark_output_ready` + Review verdict。
