# Review

审阅账本。记录某个 Review Assignment 对某个目标 Assignment 的 verdict，不判定最终是否通过。

## 核心规则

| 规则 | 内容 |
|------|------|
| Review Agent 也是市场 Agent | 它在 Registry 注册、Heartbeat 保活、Task 里参与、Assignment 里承担审查工作 |
| 审查绑定 Assignment | verdict 必须指向 `target_assignment_id` |
| 审查工作也有 Assignment | reviewer 自己的工作由 `review_assignment_id` 表达 |
| 裁决只追加不可改 | 一旦 submit，不可覆盖 |
| Failed = 重做信号 | 执行 Assignment 的 hold 不解冻，发布者决定重做或换人 |

## 与 Chain 的关系

**没有 Chain 组件**。任务链路是发起者自己管理的。Review 只知道：

```text
review_assignment_id 审查了 target_assignment_id，并提交了 verdict
```

Review 不知道这些 Assignment 在完整链路中的先后关系。

## 原语

| 原语 | 说明 |
|------|------|
| `request(task_id, target_assignment_id, review_assignment_ids, criteria)` | 创建审阅会话 |
| `submit(review_id, artifact_evidence, verdict)` | Review Agent 提交裁决，必须证明 Review Assignment 已提交 artifact |
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
    artifact_hash: OutputHash,
    target_assignment_id: AssignmentId,
    verdict: Verdict,
    submitted_at: Timestamp,
}
```

`review_assignment_id` 对应审查 Agent 自己的工作。`target_assignment_id` 对应被审查的工作。`artifact_hash` 是 Review Assignment 在 LiveSession 中提交的 ArtifactManifest hash。

## Verdict

```rust
struct Verdict {
    kind: VerdictKind,   // Passed / Failed / ArtifactUnavailable / HashMismatch
    score_bps: u16,      // 0-10000
    feedback: String,
}
```

## 结算关系

```text
Review Agent 提交 verdict
  -> 必须先 submit_artifact(review_assignment_id, ...)
  -> Review Assignment 完成
  -> reviewer hold 可以 release

目标 Assignment 的所有必要 Review 都 Passed
  -> executor hold 可以 release
```

Review 不直接放款。Settlement 执行资金变化，发布者 Agent 或后续 Policy 提供 release evidence。

## 重做流程

```text
R2 对 target_assignment_1 提交 Failed
  -> target_assignment_1 的 executor hold 保持 Active
  -> 发布者决定: 重做 or 换人
  -> 如果重做: 创建新的 Execute Assignment 和新的 Review Assignments
  -> 旧 ReviewSession 保留为历史
```

## 不是 Review 的事

- 不判定最终过没过（发布者或 Policy 判断）
- 不替发布者找 Review Agent（Registry 查）
- 不拉取 artifact 内容（Review Agent 自己拉）
- 不管理链路关系
- 不执行结算
