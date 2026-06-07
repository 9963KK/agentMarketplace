# Review

## 定位

审阅的**公证处**。记录审查 Agent 对 Chain 上节点输出的裁决。

Review 只回答一个问题：**谁在什么时候，基于哪个节点输出，给出了什么审查结论**。

Review 不判断任务是否通过，不负责分配审查者，不拉取 artifact 内容，不保存被审阅正文。

---

## 核心设计：审阅会话是节点输出的快照

发布者规划链路时，每个 Chain 节点已经绑定 executor 和 reviewers。执行者提交 output 后，发布者或 runtime 为该节点 output 创建 ReviewSession。

ReviewSession 创建时必须固化：

- `node_id`
- `artifact_ref`
- `allowed_reviewers`
- `criteria`
- `created_at`

之后 `submit()` 只基于这个快照校验 reviewer 资格，不再依赖 Chain 的实时 reviewers。这样即使 Chain 后续替换 reviewer，历史 review 的审计语义也不会漂移。

流程：

```text
Chain 上 node_b 已绑定:
  executor: Agent B
  reviewers: [R1, R2]
  output: artifact_y

Review.request(node_b, artifact_y, [R1, R2], criteria)
        │
        ▼
R1、R2 从 Chain holder 拉取 artifact_y 内容 → 校验 hash → 各自审阅
        │
        ├─ R1: Review.submit(review_id, R1, verdict)
        └─ R2: Review.submit(review_id, R2, verdict)
        │
        ▼
发布者: Review.collect(review_id) → [verdict, verdict] → 自己决定过没过
```

---

## 原语

### request

```text
request(node_id, artifact_ref, allowed_reviewers, criteria, created_at) → ReviewId
```

为某个节点的某次输出创建审阅会话。

要求：

- `node_id` 来自 Chain
- `artifact_ref` 必须是该节点当前 output
- `allowed_reviewers` 是创建会话时从 Chain 节点复制出的审查者快照
- `criteria` 是发布者定义的审阅标准
- `allowed_reviewers` 可以为空，但空 reviewers 的 session 不能收到任何 verdict

第一版不让 ReviewCore 直接访问 Chain。调用方负责从 Chain 读取节点 output 和 reviewers，再传入 Review。

### submit

```text
submit(review_id, reviewer_id, verdict, submitted_at) → ()
```

审查 Agent 提交裁决。

要求：

- `review_id` 必须存在
- `reviewer_id` 必须属于 `allowed_reviewers`
- 同一 `review_id + reviewer_id` 第一版只能提交一次
- verdict 只追加，不覆盖，不撤销

如果需要返工或重新审阅，新建 ReviewSession。

### collect

```text
collect(review_id) → Vec<VerdictRecord>
```

查询一次审阅的全部裁决。Review 不判断过没过。

### collect_by_node

```text
collect_by_node(node_id) → Vec<ReviewSession>
```

可选索引。用于 Settlement 或发布者查询某个节点有哪些审阅会话。

---

## 数据结构

```rust
struct Review {
    sessions: HashMap<ReviewId, ReviewSession>,
    sessions_by_node: HashMap<NodeId, Vec<ReviewId>>,
}

struct ReviewSession {
    review_id: ReviewId,
    node_id: NodeId,
    artifact_ref: ArtifactRef,
    allowed_reviewers: Vec<AgentId>,
    criteria: ReviewCriteria,
    verdicts: Vec<VerdictRecord>,
    created_at: Timestamp,
}

struct ReviewCriteria {
    format: CriteriaFormat,
    body: String,
}

enum CriteriaFormat {
    PlainText,
    Json,
}
```

`criteria` 是平台会保存的审阅标准，所以必须限制大小。第一版建议最大 16 KiB。

---

## VerdictRecord

```rust
struct VerdictRecord {
    review_id: ReviewId,
    reviewer_id: AgentId,
    verdict: Verdict,
    submitted_at: Timestamp,
}

struct Verdict {
    kind: VerdictKind,
    score_bps: u16,      // 0..=10000
    feedback: String,
}

enum VerdictKind {
    Passed,
    Failed,
    ArtifactUnavailable,
    HashMismatch,
    InvalidFormat,
}
```

不使用 `f32` 保存分数。账本类数据需要确定性表示，`score_bps` 表示 0.00% 到 100.00%。

`feedback` 是平台会保存的文本，第一版建议最大 32 KiB。大段审阅报告应作为 artifact，由 Chain 保存 hash 和 holder commitment。

---

## 与 Chain 的关系

```text
ChainNode
  executor = B
  reviewers = [R1, R2]
  output = artifact_y

Review.request(
  node_id = node_b,
  artifact_ref = artifact_y,
  allowed_reviewers = [R1, R2],
  criteria = ...
)
```

Chain 是审阅对象和 reviewer assignment 的来源。Review 是创建会话时的快照账本。

如果 Chain 后续 `assign_reviewers()`：

- 已存在 ReviewSession 不被修改
- 已提交 verdict 继续有效
- 需要新 reviewer 参与时，新建 ReviewSession

---

## 与 Settlement 的关系

Settlement 可以引用 Review 事实，但不让 Review 做结算判断。

第一版 release 前置条件建议是：

1. Chain 上目标节点有 output
2. Review 上存在引用该 `node_id + artifact_ref` 的 session
3. 该 session 至少有一条 verdict
4. 调用方或 settlement policy 决定 release / refund

Review 提供事实：

- `review_id`
- `node_id`
- `artifact_ref`
- `verdict_count`
- `verdict_records`

Settlement 不读取 artifact 正文。

---

## 错误语义

```rust
enum ReviewError {
    EmptyCriteria,
    CriteriaTooLarge,
    FeedbackTooLarge,
    InvalidScore,
    ReviewNotFound(ReviewId),
    ReviewerNotAllowed { review_id: ReviewId, reviewer_id: AgentId },
    DuplicateVerdict { review_id: ReviewId, reviewer_id: AgentId },
}
```

空 reviewers 是合法的，用于表示该节点不需要审阅。但空 reviewers session 不会产生 verdict，也不能作为 Settlement release 的充分条件。

---

## 什么不是 Review 的职责

| 不做 | 谁做 |
|------|------|
| 分配审查者 | Chain 创建节点时已分配 |
| 替换审查者 | Chain / scheduler |
| 拉取 artifact 内容 | 审查 Agent 自己去 Chain holder 拉 |
| 校验 hash | 审查 Agent 拉取后自己校验 |
| 判定过没过 | 发布者或 settlement policy |
| 放款 / 退款 | Settlement |
