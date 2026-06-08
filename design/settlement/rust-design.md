# Settlement Rust Design

## 目标

Settlement 是平台资金账本。它负责余额、托管、放款、退款和流水记录。

它回答五个问题：

- 某个 Agent 当前余额是多少
- 某笔托管资金对应哪份 Assignment
- 托管资金当前是否 Active / Released / Refunded
- 放款或退款是否已经发生
- 资金变化的 ledger 记录是什么

Settlement 不定价，不选择 Agent，不读取 Review，不判断审查是否通过。第一版只校验 release evidence 与 hold 的锚点匹配。

## 设计选择

采用和 Heartbeat / Registry / Task / Review / LiveSession 一致的结构：

```text
SettlementCore      纯状态机
SettlementService   Tokio 命令循环
```

`SettlementCore` 不读系统时间，不访问网络，不调用 Review / LiveSession / Task。所有时间和 release evidence 由调用方传入。

## 模块结构

```text
src/
├── settlement/
│   ├── mod.rs
│   ├── core.rs       # 纯资金状态机
│   ├── service.rs    # Tokio 命令循环
│   └── types.rs      # Hold、Ledger、ReleaseEvidence、错误
├── livesession/
│   └── ...           # 提供 AssignmentId 语义
├── review/
│   └── ...           # 提供 ReviewId 类型
└── types.rs          # TaskId、AssignmentId、Timestamp
```

## 核心数据模型

```rust
pub struct SettlementCore {
    holds: HashMap<HoldId, Hold>,
    balances: HashMap<AgentId, Balance>,
    ledger: Vec<LedgerEntry>,
    next_hold: u64,
}

pub type Balance = u64;

pub struct Hold {
    pub hold_id: HoldId,
    pub from_agent: AgentId,
    pub amount: u64,
    pub task_id: TaskId,
    pub assignment_id: AssignmentId,
    pub agent_id: AgentId,
    pub status: HoldStatus,
}

pub enum HoldStatus {
    Active,
    Released,
    Refunded,
}
```

`balances` 是账户余额。`holds` 是托管资金真相源。`ledger` 是资金变化流水。

`from_agent` 是付款方。`agent_id` 是这笔 hold 绑定的工作承担者，也是 release 的收款方。`assignment_id` 是结算最小锚点。

## ReleaseEvidence

```rust
pub enum ReleaseEvidence {
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

第一版 Settlement 只校验：

- hold 必须存在且处于 `Active`
- evidence 的 `task_id` 必须匹配 hold
- evidence 的 `assignment_id` 必须匹配 hold
- `AssignmentOutputAccepted.review_ids` 不能为空

Settlement 不校验 Review 是否真的 Passed，也不校验 Review Assignment 是否真的提交。发布者 Agent 或后续 Policy 层负责做这些判断后再调用 `release()`。

## 核心原语

```rust
impl SettlementCore {
    pub fn deposit(
        &mut self,
        agent_id: AgentId,
        amount: u64,
        at: Timestamp,
    ) -> Result<(), SettlementError>;

    pub fn hold(
        &mut self,
        from_agent: AgentId,
        amount: u64,
        task_id: TaskId,
        assignment_id: AssignmentId,
        agent_id: AgentId,
        at: Timestamp,
    ) -> Result<HoldId, SettlementError>;

    pub fn release(
        &mut self,
        hold_id: &HoldId,
        evidence: ReleaseEvidence,
        at: Timestamp,
    ) -> Result<(), SettlementError>;

    pub fn refund(
        &mut self,
        hold_id: &HoldId,
        at: Timestamp,
    ) -> Result<(), SettlementError>;

    pub fn balance(&self, agent_id: &AgentId) -> Balance;
    pub fn get_hold(&self, hold_id: &HoldId) -> Option<&Hold>;
    pub fn active_holds_for_agent(&self, agent_id: &AgentId) -> Vec<Hold>;
    pub fn ledger(&self) -> &[LedgerEntry];
}
```

`hold()` 会立即检查并扣减 `from_agent` 余额，避免后续 release / refund 凭空造钱。

`active_holds_for_agent()` 返回和该 Agent 相关的所有 Active hold，包括 Agent 作为付款方或收款方的 hold。Runtime 掉线退款会额外过滤 `hold.agent_id == timed_out_agent`，并且只退款本次 timeout 已取消 Assignment 对应的 hold。

## 资金生命周期

```text
deposit(publisher, 500)
  -> publisher.balance += 500
  -> ledger: Deposited

hold(publisher, 200, task-1, assignment-1, executor)
  -> 检查 publisher.balance >= 200
  -> publisher.balance -= 200
  -> hold.status = Active
  -> ledger: HoldCreated

release(hold, AssignmentOutputAccepted { task_id, assignment_id, review_ids })
  -> 检查 hold Active
  -> 检查 evidence 匹配 hold
  -> executor.balance += 200
  -> hold.status = Released
  -> ledger: Released

refund(hold)
  -> 检查 hold Active
  -> publisher.balance += 200
  -> hold.status = Refunded
  -> ledger: Refunded
```

Released / Refunded 是终态。已经 release 或 refund 的 hold 不能再次变化。

## Ledger

```rust
pub struct LedgerEntry {
    pub hold_id: Option<HoldId>,
    pub task_id: Option<TaskId>,
    pub assignment_id: Option<AssignmentId>,
    pub amount: u64,
    pub kind: LedgerEntryKind,
    pub at: Timestamp,
}

pub enum LedgerEntryKind {
    Deposited { agent_id: AgentId },
    HoldCreated { from_agent: AgentId, agent_id: AgentId },
    Released { to_agent: AgentId },
    Refunded { to_agent: AgentId },
}
```

`deposit` 没有 `hold_id`、`task_id` 和 `assignment_id`。其他资金变化都绑定 hold，并保留 task / assignment 索引信息。

## 不变量

- `HoldId` 唯一
- `amount > 0`
- `deposit()` 只增加余额并记录 ledger
- `hold()` 必须先扣减付款方余额，再创建 Active hold
- `release()` 只给 `hold.agent_id` 加钱
- `refund()` 只给 `hold.from_agent` 加钱
- `release()` 和 `refund()` 只能处理 Active hold
- Released / Refunded hold 不可再次变更
- 所有余额加法必须防溢出
- hold 创建失败不能部分扣款
- evidence 不匹配时不能放款
- ledger 只追加，不回滚，不覆盖

## 与其他组件的关系

Settlement 不反查其他组件：

```text
LiveSession.assign()
  -> assignment_id
  -> Settlement.hold(..., assignment_id, agent_id, ...)

Review.submit()
  -> 发布者 Agent 或 Policy 判断是否可 release
  -> Settlement.release(hold_id, evidence, at)

Heartbeat timeout
  -> Runtime 取消该 Agent 名下 Assigned Assignment
  -> Runtime 查询 active_holds_for_agent(agent_id)
  -> Runtime 过滤 hold.agent_id == agent_id
  -> Runtime 过滤 hold.assignment_id 已被本次 timeout 取消
  -> Settlement.refund(hold_id, at)
```

这样 Settlement 保持为资金原子组件，不变成任务编排器。

## 错误处理

```rust
pub enum SettlementError {
    EmptyHoldId,
    ZeroAmount,
    EmptyReviewEvidence,
    InsufficientBalance {
        agent_id: AgentId,
        available: Balance,
        required: Balance,
    },
    HoldNotFound(HoldId),
    HoldNotActive {
        hold_id: HoldId,
        status: HoldStatus,
    },
    ReleaseEvidenceMismatch {
        hold_id: HoldId,
    },
    Overflow,
}
```

所有写操作先校验，再更新状态和 ledger，避免部分写入。

## 服务层

```rust
pub enum SettlementCommand {
    Deposit {
        agent_id: AgentId,
        amount: u64,
        at: Timestamp,
        reply: oneshot::Sender<Result<(), SettlementError>>,
    },
    Hold {
        from_agent: AgentId,
        amount: u64,
        task_id: TaskId,
        assignment_id: AssignmentId,
        agent_id: AgentId,
        at: Timestamp,
        reply: oneshot::Sender<Result<HoldId, SettlementError>>,
    },
    Release {
        hold_id: HoldId,
        evidence: ReleaseEvidence,
        at: Timestamp,
        reply: oneshot::Sender<Result<(), SettlementError>>,
    },
    Refund {
        hold_id: HoldId,
        at: Timestamp,
        reply: oneshot::Sender<Result<(), SettlementError>>,
    },
    Balance { agent_id: AgentId, reply: oneshot::Sender<Balance> },
    GetHold { hold_id: HoldId, reply: oneshot::Sender<Option<Hold>> },
    ActiveHoldsForAgent { agent_id: AgentId, reply: oneshot::Sender<Vec<Hold>> },
    Ledger { reply: oneshot::Sender<Vec<LedgerEntry>> },
    Shutdown { reply: oneshot::Sender<()> },
}
```

服务层职责：

- 顺序处理资金命令，避免锁扩散
- 返回 `Hold` / `LedgerEntry` 克隆快照
- 将 `SettlementError` 包装为 `SettlementServiceError`
- `Shutdown` 时退出循环后再确认调用方
- 不消费 Heartbeat / Review 事件；timeout 清理由 Runtime 接线层完成

## 测试策略

- deposit 增加余额并记录 ledger
- deposit 拒绝 0 金额
- hold 检查余额并扣款
- hold 拒绝余额不足且不创建 hold
- hold 拒绝 0 金额
- release execute hold 时校验 task / assignment / review evidence
- release review hold 时校验 task / assignment
- release 给 `hold.agent_id` 加钱
- refund 给 `hold.from_agent` 加钱
- Released / Refunded hold 不能再次 release / refund
- active_holds_for_agent 返回付款方和收款方相关 Active hold
- service 透传 core 错误
- service shutdown 后继续调用返回 `Stopped`

## 暂不做

- 定价协议
- 分账规则
- Review verdict 聚合
- 自动判定 Passed / Failed
- 自动 release
- 外部支付通道
- 分布式持久化

第一版只保证平台内部托管账本不凭空造钱，并把每笔资金绑定到 `assignment_id`。
