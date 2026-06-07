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
  -> B 保存 A 的输入
  -> Chain.append_node(A -> B, input_hash)

B 输出给 C
  -> C 保存 B 的输出
  -> Chain.submit_output(B, output_hash)
  -> Chain.append_node(B -> C, input_hash=output_hash)

C 输出回 A
  -> A 保存最终输出
  -> Chain.submit_output(C, final_hash)
  -> Chain.close_chain(C -> A)
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
