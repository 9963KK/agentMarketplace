# Chain

## 定位

任务链路账本。记录任务如何从 Agent 流向 Agent，以及每个节点由谁执行、谁审查。

---

## 两类角色，同时分配

每个节点在创建时就确定两类角色：

```
ChainNode {
    executor: AgentId,           // 执行者 — 产出 artifact
    reviewers: Vec<AgentId>,     // 审查者 — 审阅 artifact
    input: ArtifactRef,
    output: Option<ArtifactRef>,
    ...
}
```

发布者在规划链路时，通过 Registry 同时选好执行者和审查者，再调 `append_node` 创建节点。审查不是事后补的，是链路的一部分。

---

## 原语

### create_chain

```
create_chain(task_id, root_agent_id, reviewers) → ChainId
```

创建任务链。`root_agent_id` 是发起者，`reviewers` 是根节点的审查者列表。根节点本身也是链路起点。

### append_node

```
append_node(chain_id, previous_node_id, executor, reviewers, input_artifact) → NodeId
```

追加节点。`executor` 是执行者，`reviewers` 是审查者列表。`input_artifact` 必须等于上一个节点的 output。

审查者可以为空（某些节点不需要审查），但执行者不能为空。

第一版仍然要求只能追加到当前 head，避免链路分叉。

### submit_output

```
submit_output(node_id, output_ref) → ()
```

记录节点输出。执行者产出 artifact 后调用。

要求：

- `output_ref` 必须已注册 artifact manifest
- 同一个节点只能提交一次 output
- 平台只记录 artifact 引用和 hash，不保存正文

### assign_executor

```
assign_executor(node_id, executor) → ()
```

替换节点执行者。主要用于执行者掉线、拒绝任务或长时间无输出。

第一版只允许在节点未提交 output 前替换 executor。节点已经有 output 后，executor 不再修改，避免历史产出归属漂移。

### assign_reviewers

```
assign_reviewers(node_id, reviewers) → ()
```

补充或替换审查者。Agent 掉线后可能需要更换审查者。

第一版语义：

- 替换的是 Chain 节点上的当前 reviewer assignment
- 已创建的 ReviewSession 不受影响，因为 Review 会保存 reviewers 快照
- 如果需要新 reviewer 审旧 output，应创建新的 ReviewSession

### close_chain

```
close_chain(chain_id) → ()
```

闭合链路。Chain 只检查链路结构和节点 output，不直接检查 Review 是否完成。

业务上“审查完成后才能结算”由 Settlement 或上层 runtime 检查 Review 记录。这样 Chain 不需要依赖 Review，组件边界更轻。

### get_chain

```
get_chain(chain_id) → ChainSnapshot
```

查询完整链路，含每个节点的 executor、reviewers、artifact 引用。

---

## 数据结构

```rust
struct ChainNode {
    node_id: NodeId,
    chain_id: ChainId,
    executor: AgentId,
    reviewers: Vec<AgentId>,
    previous: Option<NodeId>,
    next: Option<NodeId>,
    input: Option<ArtifactRef>,
    output: Option<ArtifactRef>,
    status: NodeStatus,
}
```

---

## 链路示例

```
Node 1                    Node 2                    Node 3
┌──────────────────┐     ┌──────────────────┐     ┌──────────────────┐
│ executor: B       │     │ executor: C       │     │ executor: D       │
│ reviewers: [R1,R2]│ ──► │ reviewers: [R1]   │ ──► │ reviewers: [R2]   │
│ input: (from A)   │     │ input: artifact_y  │     │ input: artifact_z  │
│ output: artifact_x│     │ output: artifact_y │     │ output: artifact_f │
└──────────────────┘     └──────────────────┘     └──────────────────┘
```

---

## 与 Registry 的关系

发布者规划链路：

```
discover("code-analysis")       → 选 executor
discover("review:code-analysis") → 选 reviewers
chain.append_node(executor, reviewers, input)
```

---

## 与 Review 的关系

节点创建时审查者已分配。执行者产出 artifact 后，发布者或 runtime 用节点上的 reviewers 和 output 创建 ReviewSession。

```
ChainNode.output + ChainNode.reviewers
  -> Review.request(node_id, artifact_ref, reviewers_snapshot, criteria)
```

Review 记录的是创建会话时的快照。后续 `assign_reviewers()` 不会修改已有 ReviewSession。

---

## Agent 掉线处理

- **执行者掉线**：Heartbeat → Registry 移除 → 发布者通过 Registry 选新执行者 → `assign_executor()` → 继续
- **审查者掉线且未创建 ReviewSession**：Heartbeat → Registry 移除 → 发布者调 `assign_reviewers()` 替换
- **审查者掉线且已创建 ReviewSession**：旧 session 保留；如果需要替换审阅，新建 ReviewSession

Chain 不主动订阅 Heartbeat，也不自动改派。掉线事件由上层 runtime / scheduler 消费后调用 Chain。

---

## 平台存储边界

平台存：chain / node / executor / reviewers / artifact hash / holder 承诺

平台不存：artifact 正文、Agent 间通信内容
