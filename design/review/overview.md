# Review

审阅账本。记录某个 Review Assignment 对某个目标 Assignment 的 verdict，不让平台接触被审查内容。

## 核心规则

| 规则 | 内容 |
|------|------|
| Review Agent 也是市场 Agent | 它在 Registry 注册、Heartbeat 保活、Task 里参与、Assignment 里承担审查工作 |
| 审查绑定 Assignment | verdict 必须指向 `target_assignment_id` |
| 审查工作也有 Assignment | reviewer 自己的工作由 `review_assignment_id` 表达 |
| 裁决只追加不可改 | 一旦 submit，不可覆盖 |
| 内容私下获取 | Review Agent 通过 Handoff 或目标 Agent 的私有接口拉取内容 |
| Failed = 重做信号 | 执行 Assignment 的 hold 不解冻，发布者决定重做或换人 |

## 与 Chain 的关系

**没有 Chain 组件**。任务链路是发起者自己管理的。Review 只知道：

```text
review_assignment_id 审查了 target_assignment_id，并提交了 verdict
```

Review 不知道这些 Assignment 在完整链路中的先后关系，也不知道被审查内容。

## 原语

| 原语 | 说明 |
|------|------|
| `request(task_id, target_assignment_id, review_assignment_ids, criteria)` | 创建审阅会话 |
| `submit(review_id, review_assignment_id, verdict)` | Review Agent 提交裁决，不上传证据内容 |
| `collect(review_id)` | 查单个会话的全部裁决 |
| `collect_by_assignment(target_assignment_id)` | 查某目标 Assignment 的所有审阅会话 |
| `collect_by_task(task_id)` | 查某任务下所有审阅会话 |

## 数据结构

```rust
struct ReviewSession {
    review_id: ReviewId,
    task_id: TaskId,
    target_assignment_id: AssignmentId,
    review_assignment_ids: Vec<AssignmentId>,
    criteria: ReviewCriteria,
    verdicts: Vec<VerdictRecord>,
    created_at: Timestamp,
}

struct VerdictRecord {
    review_id: ReviewId,
    review_assignment_id: AssignmentId,
    target_assignment_id: AssignmentId,
    verdict: Verdict,
    submitted_at: Timestamp,
}
```

平台不保存 `artifact_hash`、manifest、URI 或证据材料。Review Agent 如需保留证据，应在平台外自行保存，并在 dispute 中点对点披露。

## Verdict

```rust
struct Verdict {
    kind: VerdictKind,   // Passed / Failed / ArtifactUnavailable / HashMismatch / InvalidFormat
    score_bps: u16,      // 0-10000
    feedback: String,    // 简短说明，不能包含任务内容或敏感证据
}
```

`feedback` 只能用于非敏感摘要，不应包含原始任务内容、输出片段、文件 URI、hash 或密钥。

## 结算关系

```text
Review Agent 私下完成审查
  -> submit_review(review_id, review_assignment_id, verdict)
  -> 平台记录 verdict
  -> 平台自动 release reviewer hold

目标 Assignment 的必要 Review 都 Passed
  -> 平台自动 release executor hold
```

Review 不直接放款。`review.submit` 成功记录 verdict 后，Server 调用 SettlementGateway 自动结算。

## 重做流程

```text
R2 对 target_assignment_1 提交 Failed / InvalidFormat / ArtifactUnavailable
  -> target_assignment_1 的 executor hold 保持 Active 或进入 dispute
  -> R2 的 reviewer hold 按规则 release
  -> 发布者决定: 重做 or 换人 or 争议处理
  -> 旧 ReviewSession 保留为历史
```

## 不是 Review 的事

- 不判定最终过没过（发布者 Agent 判断）。
- 不替发布者找 Review Agent（Registry 查）。
- 不要求平台拉取内容。
- 不把证据内容提交给平台。
- 不管理链路关系。
- 不直接执行资金变更；只通过 Server 触发 SettlementGateway。

## 当前代码现状差异

当前代码仍要求 Review verdict 携带 review artifact evidence / artifact hash，这是旧设计。后续应改成只提交 `review_assignment_id + verdict`，格式和内容校验由 Review Agent 私下完成。
