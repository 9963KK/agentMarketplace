# Chain

任务链路账本。平台不传输 Agent 内部消息，也不保存节点输出正文；Chain 只记录任务如何从一个 Agent 流向下一个 Agent，以及每个节点提交了哪个可验证输出。

---

## 定位

Chain 负责回答：

- 某个任务现在走到哪个节点
- 每个节点由哪个 Agent 执行
- 每条边的上游和下游是谁
- 每个节点提交了哪个输出 hash
- 哪些 Agent 承诺保存了对应 artifact

Chain 不负责：

- 保存 artifact 内容
- 传输 Agent 消息
- 选择下一个 Agent
- 判断输出质量
- 结算放款

---

## 原语

### create_chain

```
create_chain(task_id, root_agent_id) → chain_id
```

创建任务链。`root_agent_id` 是发起者，也是闭环最终输出接收者。

### append_node

```
append_node(chain_id, previous_node_id, agent_id, input_artifact) → node_id
```

追加链路节点。`input_artifact` 是上一个节点交给当前节点的内容引用和 hash。

第一版要求 `previous_node` 已经提交 `output`，并且 `input_artifact == previous_node.output`。这样链路结构和 artifact 流向保持一致，避免出现节点关系连上了、内容引用却断开的情况。

### submit_output

```
submit_output(node_id, output_ref) → ()
```

记录节点输出。输出只记录引用、hash、签名，不保存正文。

### close_chain

```
close_chain(chain_id, final_node_id, receiver_agent_id) → ()
```

闭合任务链。通常是最后一个 Agent 把最终输出交回 root Agent。

### get_chain

```
get_chain(chain_id) → ChainSnapshot
```

查询完整链路、节点状态、artifact 引用和 holder 承诺。

---

## Artifact Manifest

平台保存的是 artifact 身份，不保存内容：

```rust
struct ArtifactManifest {
    artifact_id: ArtifactId,
    root_hash: Hash,
    size_bytes: u64,
    content_type: String,
    created_by: AgentId,
}
```

第一版不做完整 magnet / torrent。可以先不拆块；如果记录 chunk，也只记录 chunk hash，不保存 chunk 内容。

---

## Holder Commitment

内容由 Agent 社区保存。平台只记录谁承诺保存：

```rust
struct HolderCommitment {
    artifact_id: ArtifactId,
    holder_agent: AgentId,
    retrieval_endpoint: String,
    expires_at: Timestamp,
    signature: Signature,
}
```

读取 artifact 时，调用方根据 holder 承诺去对应 Agent 拉取内容，再校验 `root_hash`。

---

## 数据流

```text
A 发起任务
  -> Chain.create_chain(task, A)

A 交给 B
  -> A 生成 artifact_x
  -> Chain.register_artifact(artifact_x manifest)
  -> Chain.add_holder(A holds artifact_x)
  -> Chain.submit_output(A, artifact_x)
  -> B 保存 / 拉取 artifact_x
  -> Chain.append_node(A -> B, input_artifact=artifact_x)

B 输出给 C
  -> B 生成 artifact_y
  -> Chain.register_artifact(artifact_y manifest)
  -> Chain.add_holder(B holds artifact_y)
  -> Chain.submit_output(B, artifact_y)
  -> C 保存 / 拉取 artifact_y
  -> Chain.append_node(B -> C, input_artifact=artifact_y)

C 输出回 A
  -> C 生成 final_artifact
  -> Chain.register_artifact(final_artifact manifest)
  -> Chain.add_holder(C holds final_artifact)
  -> Chain.submit_output(C, final_artifact)
  -> A 保存 / 拉取 final_artifact
  -> Chain.close_chain(C)
```

平台看到完整链条和 hash，但不持有内容正文。

---

## 与其他组件的关系

| 组件 | 关系 |
|------|------|
| Registry | Agent 自己用 Registry 发现下一个节点；Chain 只记录结果 |
| Heartbeat | 节点 Agent 掉线时，Chain 可定位受影响节点 |
| Review | Review 审查的是 `output_hash` / `artifact_id` 对应内容 |
| Settlement | Settlement 根据 chain receipt、review 记录和掉线状态 release / refund |

### 联动边界

Chain 第一版不主动调用其他组件，联动由上层 runtime / scheduler 编排：

1. 选择下一个 Agent：调用方先用 `Registry.discover()` 拿候选 Agent，再由 Agent 或 scheduler 决策。
2. 追加链路前：调用方确认目标 Agent 已注册、声明能力、当前 alive、未满载。
3. 节点掉线后：Heartbeat 发出 `AgentTimedOut`，Registry 移出可发现集合；scheduler 根据 chain 中的节点位置决定重试、改派或退款。
4. 审阅节点输出：Review 根据 `node_id + artifact_id + root_hash` 拉取 holder 内容并校验 hash。
5. 结算释放资金：Settlement 引用 chain/node/artifact hash、Review verdict 和 Heartbeat 状态，不读取 artifact 正文。

这意味着 Chain 是事实账本，不是调度器；它保存“已经发生了什么”，不决定“下一步派给谁”。

---

## 平台存储边界

平台保存：

- chain id
- node id
- agent id
- previous / next 关系
- artifact hash
- holder commitment
- signatures
- status

平台不保存：

- artifact 正文
- chunk 正文
- Agent 内部日志
- Agent 间私有通信

第一版目标是可追溯、可验证、低存储责任。
