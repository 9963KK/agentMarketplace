# Review

审阅公证处。只记录审查裁决，不判定过没过。

## 核心规则

| 规则 | 内容 |
|------|------|
| 每个产出必须被审 | Settlement 放款前提 |
| 裁决只追加不可改 | 一旦 submit，不可覆盖 |
| Failed = 重做权 | 执行者拿不到钱，发布者决定重做或换人 |

## 与 Chain 的关系

**没有 Chain 组件**。任务链路是发起者自己管理的。Review 只知道 `task_id`、`executor`、`reviewer` 和 `verdict`，不知道节点之间怎么连。

发布者拿 `collect_by_task(task_id)` 的结果自己判断。

## 原语

| 原语 | 说明 |
|------|------|
| `request(task_id, executor_id, output_hash, criteria, reviewers)` | 创建审阅会话 |
| `submit(review_id, reviewer_id, verdict)` | 审查者提交裁决 |
| `collect(review_id)` | 查单个会话的全部裁决 |
| `collect_by_task(task_id)` | 查某任务的所有审阅会话 |

## Verdict

```rust
struct Verdict {
    kind: VerdictKind,   // Passed / Failed / ArtifactUnavailable / HashMismatch
    score_bps: u16,      // 0-10000
    feedback: String,
}
```

## 重做流程

```
R2 交 Failed → 执行者 hold 不解冻
  → 发布者决定: 重做 or 换人
  → 如果重做: Review.request(新的 session)
  → 旧 session 保留为历史
  → Settlement 只看最新 session 结果
```

## 不是 Review 的事

- 不判定过没过（发布者自己判）
- 不替发布者找审查者（Registry 查）
- 不拉取 artifact 内容（审查 Agent 自己拉）
- 不管理链路关系
