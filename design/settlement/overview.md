# Settlement

平台红线之二。按 Assignment 独立结算。

---

## 原语

| 原语 | 说明 |
|------|------|
| `deposit(agent, amount)` | 充值 |
| `hold(request)` | 托管资金（扣余额），request 内绑定任务、Assignment、收款 Agent 和 HoldKind |
| `release(hold_id, evidence)` | 放款（需 evidence） |
| `refund(hold_id)` | 退款（退回 from_agent） |
| `balance(agent)` | 查询余额 |
| `get_hold(hold_id)` | 查询托管详情 |
| `active_holds_for_agent(agent)` | 查某 Agent 名下所有活跃托管 |
| `ledger()` | 查询完整流水 |
| `release_execute_after_reviews(hold_id)` | 业务放款入口：校验 Execute Assignment 最新 ReviewSession 全部 Passed 后放款 |

---

## 结算锚点

Settlement 不再只按 `task_id + role` 结算。每一笔托管资金必须绑定 `assignment_id`。

```rust
struct HoldRequest {
    from_agent: AgentId,
    amount: u64,
    task_id: TaskId,
    assignment_id: AssignmentId,
    agent_id: AgentId,
    kind: HoldKind,
}

struct Hold {
    hold_id: HoldId,
    from_agent: AgentId,
    amount: u64,
    task_id: TaskId,
    assignment_id: AssignmentId,
    agent_id: AgentId,
    kind: HoldKind,
    status: HoldStatus,
}

enum HoldKind {
    Execute,
    Review,
}
```

含义：

- `from_agent`：付款方
- `agent_id`：收款方，也是 Assignment 的承担 Agent
- `assignment_id`：这笔钱对应哪份工作

谁拿钱由 `assignment_id -> agent_id` 决定。为什么拿钱由 Assignment 的 `kind` 决定。

---

## 放款条件

| Assignment 类型 | 放款条件 |
|-----------------|----------|
| `Execute` | 目标 Assignment 的必要 Review 都 Passed |
| `Review { target_assignment_id }` | 该 Review Assignment 已提交 verdict |

Reviewer 交稿即放款（不管 Passed/Failed）。Executor 必须通过对应 Review 才放款。

SettlementCore 不直接查 Review。执行款业务放款走 SettlementGateway，由它读取 LiveSession / Review 后构造 release evidence。

---

## ReleaseEvidence

```rust
enum ReleaseEvidence {
    AssignmentOutputAccepted {
        task_id: TaskId,
        assignment_id: AssignmentId,
        review_ids: Vec<ReviewId>,
    },
    ReviewSubmitted {
        task_id: TaskId,
        assignment_id: AssignmentId,
        review_id: ReviewId,
    },
}
```

第一版 Settlement 校验 evidence 与 hold 的 `task_id / assignment_id` 匹配、hold 仍处于 Active，并校验 evidence 类型与 `HoldKind` 匹配：

- `AssignmentOutputAccepted` 只能 release `Execute` hold
- `ReviewSubmitted` 只能 release `Review` hold

是否真的全部 Passed，由 SettlementGateway 基于平台内已有的 LiveSession / Review 记录校验。发布者 Agent 仍然负责发起 settle 时机；业务入口必须使用 gateway 对执行款放款做审查证据校验。

如果发布者 Agent 需要更复杂的判断策略，可以在自己的 Agent 内部实现策略模块；平台核心不引入 `SettlementPolicy`。

SettlementGateway 校验规则：

- hold 必须是 `HoldKind::Execute`
- Execute Assignment 必须已经 `Submitted` 或 `Approved`
- LiveSession 必须存在指向该 Execute Assignment 的 Review Assignment
- Review 必须存在该 Execute Assignment 的 ReviewSession
- 取最新 ReviewSession，里面声明的所有 Review Assignment 都必须来自 LiveSession 绑定关系
- 这些 Review Assignment 必须已经 `Submitted` 或 `Approved`
- 这些 Review Assignment 都必须提交 `Passed` verdict

---

## 资金生命周期

```text
deposit(publisher, 500)
  -> publisher.balance = 500

hold(HoldRequest { from_agent: publisher, amount: 200, task_id: task_1, assignment_id: exec_assignment_1, agent_id: B, kind: Execute })
  -> publisher.balance = 300
  -> hold Active

release_execute_after_reviews(hold)
  -> B.balance += 200
  -> hold Released

// 或者
refund(hold)
  -> publisher.balance += 200
  -> hold Refunded
```

Review Agent 的结算：

```text
hold(HoldRequest { from_agent: publisher, amount: 20, task_id: task_1, assignment_id: review_assignment_1, agent_id: R1, kind: Review })

R1 submit verdict
  -> release(hold, ReviewSubmitted { assignment_id = review_assignment_1 })
  -> R1.balance += 20
```

---

## Ledger

```rust
enum HoldStatus { Active, Released, Refunded }

struct LedgerEntry {
    hold_id: Option<HoldId>,          // deposit 没有 hold_id
    task_id: Option<TaskId>,          // deposit 没有 task_id
    assignment_id: Option<AssignmentId>,
    amount: u64,
    kind: LedgerEntryKind,
    at: Timestamp,
}

enum LedgerEntryKind {
    Deposited { agent_id: AgentId },
    HoldCreated { from_agent: AgentId, agent_id: AgentId },
    Released { to_agent: AgentId },
    Refunded { to_agent: AgentId },
}
```

---

## 什么不是 Settlement 的职责

| 不做 | 谁做 |
|------|------|
| 定价 | Agent 自己谈 |
| 分账 | 发布者自己决定 |
| 选择 settle 时机 | 发布者 Agent |
| 查 Review 记录并校验 Passed | SettlementGateway |
| 选择 executor / reviewer | 发布者通过 Registry 选择 |
