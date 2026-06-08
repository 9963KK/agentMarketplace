# Review Rust Design

## 目标

Review 是审阅账本。它记录审查 Agent 对某个任务输出 hash 的 verdict。

Review 不保存 artifact 内容，不拉取内容，不校验 hash，不判断最终通过，不做结算。

## 设计选择

采用和 Heartbeat / Registry 一致的结构：

```text
ReviewCore      纯状态机
ReviewService   Tokio 命令循环
```

`ReviewCore` 不读系统时间，不访问网络。调用方在 `request()` 时传入 `task_id`、`executor_id`、`output_hash` 和 reviewers 快照。

## 模块结构

```text
src/
├── review/
│   ├── mod.rs
│   ├── core.rs
│   ├── service.rs
│   └── types.rs
├── types.rs
├── registry/
├── heartbeat/
└── settlement/
```

## 数据模型

```rust
pub struct ReviewCore {
    sessions: HashMap<ReviewId, ReviewSession>,
    sessions_by_task: HashMap<TaskId, Vec<ReviewId>>,
    next_review: u64,
}

pub struct ReviewSession {
    pub review_id: ReviewId,
    pub task_id: TaskId,
    pub executor_id: AgentId,
    pub output_hash: OutputHash,
    pub allowed_reviewers: Vec<AgentId>,
    pub criteria: ReviewCriteria,
    pub verdicts: Vec<VerdictRecord>,
    pub created_at: Timestamp,
}
```

ReviewSession 是创建时快照。任务链路由发起 Agent 自己管理，平台不保存节点关系。

## Verdict

```rust
pub struct VerdictRecord {
    pub review_id: ReviewId,
    pub reviewer_id: AgentId,
    pub verdict: Verdict,
    pub submitted_at: Timestamp,
}

pub struct Verdict {
    pub kind: VerdictKind,
    pub score_bps: u16,
    pub feedback: String,
}

pub enum VerdictKind {
    Passed,
    Failed,
    ArtifactUnavailable,
    HashMismatch,
    InvalidFormat,
}
```

`score_bps` 取值范围是 `0..=10000`。不用 `f32`，避免账本数据出现 NaN、精度和序列化问题。

## 核心原语

```rust
impl ReviewCore {
    pub fn request(
        &mut self,
        task_id: TaskId,
        executor_id: AgentId,
        output_hash: OutputHash,
        allowed_reviewers: Vec<AgentId>,
        criteria: ReviewCriteria,
        created_at: Timestamp,
    ) -> Result<ReviewId, ReviewError>;

    pub fn submit(
        &mut self,
        review_id: &ReviewId,
        reviewer_id: AgentId,
        verdict: Verdict,
        submitted_at: Timestamp,
    ) -> Result<(), ReviewError>;

    pub fn collect(&self, review_id: &ReviewId) -> Option<Vec<VerdictRecord>>;
    pub fn collect_by_task(&self, task_id: &TaskId) -> Vec<ReviewSession>;
}
```

`created_at` 和 `submitted_at` 由调用方或 service 层传入，Core 不读取系统时间。

## 不变量

- `ReviewSession.review_id` 唯一
- `allowed_reviewers` 内不能重复
- `submit.reviewer_id` 必须属于 `allowed_reviewers`
- 同一 `review_id + reviewer_id` 第一版只能提交一次
- verdict 只追加，不覆盖，不撤销
- `criteria.body` 不能为空且不能超过上限
- `feedback` 不能超过上限
- `score_bps <= 10000`
- Review 不管理任务链路，不修改 Settlement

## 大小限制

第一版建议：

- `criteria.body <= 16 KiB`
- `verdict.feedback <= 32 KiB`

超过限制的长文档、审查报告、证据材料不进入平台存储。

## 与 Settlement 的协作

Settlement 可以查询：

```text
Review.collect_by_task(task_id)
  -> sessions
  -> latest session
  -> verdict_count / verdict kinds
```

第一版 Settlement 可用规则：

- 没有 ReviewSession：不能 release executor
- 有 ReviewSession 但没有 verdict：不能 release executor
- executor release 使用最新 session 的 verdict 集合
- reviewer release 只要求该 reviewer 已提交 verdict

Review 不判断 passed / failed 的最终业务含义。

## 错误处理

```rust
pub enum ReviewError {
    EmptyCriteria,
    CriteriaTooLarge { max_bytes: usize, actual_bytes: usize },
    FeedbackTooLarge { max_bytes: usize, actual_bytes: usize },
    InvalidScore(u16),
    DuplicateReviewer(AgentId),
    ReviewNotFound(ReviewId),
    ReviewerNotAllowed { review_id: ReviewId, reviewer_id: AgentId },
    DuplicateVerdict { review_id: ReviewId, reviewer_id: AgentId },
}
```

所有写操作先完整校验，再更新状态，避免部分写入。

## 测试重点

- request 创建 session 并建立 task 索引
- request 拒绝空 criteria
- request 拒绝重复 reviewer
- submit 拒绝未知 review
- submit 拒绝未授权 reviewer
- submit 拒绝重复 verdict
- submit 拒绝 `score_bps > 10000`
- collect 返回不可变快照
- collect_by_task 返回该任务所有 session
