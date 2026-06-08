# LiveSession / Assignment

## 定位

LiveSession 是任务当前运行批次。Assignment 是市场 Agent 被分配的一份可追踪、可审查、可结算的工作。

平台仍然不保存链路顺序。LiveSession 不表达 DAG，不表达上游/下游，只聚合当前批次里有哪些 Assignment 正在运行。

---

## 核心原则

| 原则 | 说明 |
|------|------|
| 所有 Agent 平等 | executor、reviewer、planner、aggregator 都是市场 Agent |
| Assignment 是最小锚点 | 完成、审查、结算都绑定 assignment_id |
| Review Agent 不是附属字段 | reviewer 也有自己的 Assignment |
| 不保存链路顺序 | 不记录 A -> B -> C，不记录输入输出边 |
| 不保存 artifact | 只记录 output_hash，不存内容 |

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

enum LiveSessionStatus {
    Running,
    Closed,
}

struct Assignment {
    assignment_id: AssignmentId,
    task_id: TaskId,
    session_id: SessionId,
    agent_id: AgentId,
    kind: AssignmentKind,
    status: AssignmentStatus,
    output_hash: Option<OutputHash>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

enum AssignmentKind {
    Execute,
    Review { target_assignment_id: AssignmentId },
}

enum AssignmentStatus {
    Assigned,
    Submitted,
    Approved,
    Rejected,
    Cancelled,
}
```

`Review { target_assignment_id }` 表示这个 Review Assignment 审查的是哪份 Execute Assignment。

---

## 示例

```text
task_1
  session_1
    assignment_1: Execute, agent=B
    assignment_2: Review { target=assignment_1 }, agent=R1
    assignment_3: Review { target=assignment_1 }, agent=R2
```

含义：

- B 是市场上的执行 Agent。
- R1 / R2 是市场上的审查 Agent。
- R1 / R2 不是 B 的附属字段，它们各自有独立 Assignment。
- 平台知道 R1 / R2 审查 `assignment_1`，但不知道 `assignment_1` 在完整链路中的上游或下游。

---

## 原语

| 原语 | 说明 |
|------|------|
| `create_session(task_id, at)` | 创建当前运行批次 |
| `close_session(session_id, at)` | 关闭运行批次 |
| `assign(task_id, session_id, agent_id, kind, at)` → `AssignmentId` | 创建 Assignment |
| `submit_output(assignment_id, agent_id, output_hash, at)` | Assignment 完成并提交 output hash |
| `mark_approved(assignment_id, at)` | 标记 Assignment 审查通过 |
| `mark_rejected(assignment_id, at)` | 标记 Assignment 审查失败 |
| `cancel_assignment(assignment_id, at)` | 取消 Assignment |
| `assignments_by_task(task_id)` | 查询任务下全部 Assignment |
| `assignments_by_session(session_id)` | 查询当前批次 Assignment |
| `assignments_by_agent(agent_id)` | 查询某 Agent 的 Assignment |

---

## 完成语义

平台不能凭空知道 Agent 完成了工作。完成必须由承担该 Assignment 的 Agent 提交：

```text
submit_output(assignment_id, agent_id, output_hash)
```

校验：

- `assignment_id` 存在
- `agent_id == assignment.agent_id`
- Assignment 仍处于 `Assigned`
- `output_hash` 不为空

执行 Assignment 的输出是业务产物 hash。Review Assignment 的输出可以是 verdict hash 或审查报告 hash；具体 verdict 仍由 Review 组件记录。

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

这样平台能知道：

- 哪个市场 Agent 做了审查工作
- 它审查的是哪份执行工作
- 它自己的审查工作是否已提交
- 它自己的结算应该绑定哪份 Assignment

---

## Settlement 关系

Settlement 按 Assignment 结算。

```rust
struct Hold {
    hold_id: HoldId,
    from_agent: AgentId,
    amount: u64,
    task_id: TaskId,
    assignment_id: AssignmentId,
    agent_id: AgentId,
    status: HoldStatus,
}
```

谁拿钱由 `assignment_id -> agent_id` 决定。为什么拿钱由 `AssignmentKind` 决定。

结算规则：

```text
Review Assignment 提交 verdict
  -> release reviewer assignment 的 hold

Execute Assignment 的所有目标 Review 都 Passed
  -> release execute assignment 的 hold
```

Settlement 不判断 Review 是否通过。发布者 Agent 或后续 Policy 层提供 release evidence。

---

## 与 Task 的关系

Task 是任务容器。LiveSession / Assignment 是任务内当前工作批次和工作单元。

```text
Task
  -> LiveSession
      -> Assignment
```

Task 仍然只保存当前参与者和历史参与者。创建 Assignment 时，调用方应同步：

```text
Task.add_participant(task_id, assignment.agent_id)
```

---

## 不做的事

| 不做 | 谁做 |
|------|------|
| 链路顺序 | 发布者 Agent 自己管 |
| 上下游依赖 | 发布者 Agent 自己管 |
| artifact 内容存储 | Agent 自己存 |
| 自动选择 Agent | 发布者 Agent 通过 Registry 选择 |
| 自动判断是否通过 | 发布者 Agent 或后续 Policy |
| 自动放款 | Settlement 执行，触发者提供 evidence |

第一版只把完成、审查、结算锚定到 `assignment_id`，不引入 Chain。
