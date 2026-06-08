# Settlement

平台红线之二。按 Assignment 独立结算。

---

## 原语

| 原语 | 说明 |
|------|------|
| `deposit(agent, amount)` | 充值 |
| `hold(from, amount, task_id, assignment_id, agent_id)` | 托管资金（扣余额） |
| `release(hold_id, evidence)` | 放款（需 evidence） |
| `refund(hold_id)` | 退款（退回 from_agent） |
| `balance(agent)` | 查询余额 |
| `get_hold(hold_id)` | 查询托管详情 |
| `active_holds_for_agent(agent)` | 查某 Agent 名下所有活跃托管 |
| `ledger()` | 查询完整流水 |

---

## 结算锚点

Settlement 不再只按 `task_id + role` 结算。每一笔托管资金必须绑定 `assignment_id`。

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

Settlement 不直接查 Review。发布者 Agent 或后续 Policy 提供 release evidence。

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

第一版 Settlement 只校验 evidence 与 hold 的 `task_id / assignment_id` 匹配，以及 hold 仍处于 Active。是否真的全部 Passed，由发布者 Agent 或后续 Policy 判断后再调用 release。

后续如果要平台自动判定，可以引入 `SettlementPolicy`，但不放进第一版 SettlementCore。

---

## 资金生命周期

```text
deposit(publisher, 500)
  -> publisher.balance = 500

hold(publisher, 200, task_1, exec_assignment_1, B)
  -> publisher.balance = 300
  -> hold Active

release(hold, AssignmentOutputAccepted { assignment_id = exec_assignment_1 })
  -> B.balance += 200
  -> hold Released

// 或者
refund(hold)
  -> publisher.balance += 200
  -> hold Refunded
```

Review Agent 的结算：

```text
hold(publisher, 20, task_1, review_assignment_1, R1)

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
| 判断审阅过没过 | 发布者或后续 Policy |
| 查 Review 记录 | 发布者提供 ReleaseEvidence |
| 选择 executor / reviewer | 发布者通过 Registry 选择 |
