# Review Rust Design

## 目标

Review 是审阅账本。它记录审查 Agent 对某个 Chain 节点 output artifact 的 verdict。

Review 不保存 artifact 内容，不拉取 holder，不校验 hash，不判断最终通过，不做结算。

## 设计选择

采用和 Heartbeat / Registry / Chain 一致的结构：

```text
ReviewCore      纯状态机
ReviewService   Tokio 命令循环
```

`ReviewCore` 不读系统时间，不访问网络，不直接查询 Chain。调用方在 `request()` 时传入从 Chain 读取到的 `node_id`、`artifact_ref` 和 reviewers 快照。

## 模块结构

```text
src/
├── review/
│   ├── mod.rs
│   ├── core.rs
│   ├── service.rs
│   └── types.rs
├── chain/
├── registry/
├── heartbeat/
└── settlement/
```

## 数据模型

```rust
pub struct ReviewCore {
    sessions: HashMap<ReviewId, ReviewSession>,
    sessions_by_node: HashMap<NodeId, Vec<ReviewId>>,
    next_review: u64,
}

pub struct ReviewSession {
    pub review_id: ReviewId,
    pub node_id: NodeId,
    pub artifact_ref: ArtifactRef,
    pub allowed_reviewers: Vec<AgentId>,
    pub criteria: ReviewCriteria,
    pub verdicts: Vec<VerdictRecord>,
    pub created_at: Timestamp,
}

pub struct ReviewCriteria {
    pub format: CriteriaFormat,
    pub body: String,
}

pub enum CriteriaFormat {
    PlainText,
    Json,
}
```

ReviewSession 是创建时快照。后续 Chain 修改 reviewers 不会回写已有 session。

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
        node_id: NodeId,
        artifact_ref: ArtifactRef,
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
    pub fn collect_by_node(&self, node_id: &NodeId) -> Vec<ReviewSession>;
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
- Review 不修改 Chain 节点，不修改 Settlement

## 大小限制

第一版建议：

- `criteria.body <= 16 KiB`
- `verdict.feedback <= 32 KiB`

超过限制的长文档、审查报告、证据材料都应该作为 artifact，由 Chain 保存 hash 和 holder commitment。

## 与 Chain 的协作

调用方负责：

```text
Chain.get_chain(chain_id)
  -> 找到 node_id
  -> 读取 node.output
  -> 读取 node.reviewers
  -> Review.request(node_id, output, reviewers, criteria, now)
```

Review 不自己查询 Chain。这样可以保持 ReviewCore 可测试、可复用，也避免组件之间循环依赖。

## 与 Settlement 的协作

Settlement 可以查询：

```text
Review.collect_by_node(node_id)
  -> sessions
  -> verdict_count
```

第一版 Settlement 可用规则：

- 没有 ReviewSession：不能 release
- 有 ReviewSession 但没有 verdict：不能 release
- 是否 release 仍由调用方或 settlement policy 决定

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

- request 创建 session 并建立 node 索引
- request 拒绝空 criteria
- request 拒绝重复 reviewer
- submit 拒绝未知 review
- submit 拒绝未授权 reviewer
- submit 拒绝重复 verdict
- submit 拒绝 `score_bps > 10000`
- collect 返回不可变快照
- collect_by_node 返回该节点所有 session
