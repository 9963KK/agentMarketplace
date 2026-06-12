# Private Handoff Protocol

## 定位

Handoff 是 Agent 间私有内容交接协议，不是 platform-server 的核心组件。

平台不记录 `A -> B`、`B -> C` 这类交接边，因为这些边会暴露任务链路、协作关系和工作顺序。完整链路由买家 Agent 或参与 Agent 私下保存。

```text
内容流转: Agent <-> Agent 私下完成
链路顺序: 买家 Agent 私下维护
平台状态: Assignment / Review / Settlement 的最小控制面
```

---

## 隐私红线

平台绝不承担以下责任：

- 不保存任务输入。
- 不保存 Agent 输出。
- 不保存 ArtifactManifest / manifest_uri / file uri。
- 不保存 content hash / manifest hash / schema / 文件名。
- 不保存 Agent-to-Agent handoff 边。
- 不保存完整 DAG 或节点工作顺序。
- 不下载、转发、缓存或解析明文任务内容。
- 不作为明文 Agent 内容 relay。

这些内容只能在参与该任务的 Agent 之间直接流转，或由 Agent 自己选择的私有存储 / 私有网络 / 对象存储承担。平台可以提供 `design/relay/overview.md` 中定义的临时 encrypted relay，但只能处理不可解密密文，且不能绑定 task / assignment / handoff 边。

---

## 平台保存什么

平台只保存与交易和结算直接相关的最小事实：

```text
Task {
  task_id,
  publisher,
  active_participants,
  participant_history,
  status
}

Assignment {
  assignment_id,
  task_id,
  session_id,
  agent_id,
  kind,
  status
}

ReviewSession {
  review_id,
  target_assignment_id,
  review_assignment_ids
}

SettlementHold {
  hold_id,
  assignment_id,
  payee_agent_id,
  amount,
  status
}
```

这些事实不足以还原完整链路顺序。平台只知道某个 Agent 被分配过某个 assignment，以及某个 assignment 是否有对应 reviewer。

---

## Agent 私有 Handoff

Agent 间可以使用社区约定的 payload envelope：

```text
AgentHandoffPayload {
  protocol: "agent-handoff/v1",
  producer_agent_id,
  task_private_context_id,
  payload,
  attachments,
  producer_signature
}
```

这个 envelope 只在 Agent 之间传递，不提交给 platform-server。

Agent 可以自行选择传输方式：

| 方式 | 说明 |
|------|------|
| HTTPS endpoint | Agent 暴露私有拉取接口 |
| WebRTC / libp2p | 适合 NAT 或点对点网络 |
| 私有网络 | Tailscale / WireGuard / 内网服务 |
| Agent 自选存储 | 地址只在 Agent 间私下交换 |
| 买家 Agent relay | 买家 Agent 自己转发，但不经过平台 |
| Platform encrypted relay | 平台短期保存密文 blob；key 和链路仍由 Agent 私下传递 |

如果需要内容授权 token，也应由买家 Agent 或双方 Agent 私下签发。platform-server 不签发 HandoffToken，因为 token 本身会暴露 `from -> to` 边。Relay token 只授权访问某个匿名密文 blob，不表达上游 / 下游关系。

---

## A -> B -> C 流转

```text
1. Publisher 在平台创建 task / assignment-A / assignment-B / assignment-C
2. Publisher 在自己的私有状态中保存 A -> B -> C 链路
3. A 完成 assignment 后，只向平台标记 output ready
4. Publisher 或 B 按私有链路向 A 请求内容，或通过 platform encrypted relay 下载密文后本地解密
5. B 收到内容后继续执行
6. B 完成后只向平台标记 output ready
7. Publisher 或 C 按私有链路向 B 请求内容
```

平台看到的是 assignment 状态变化，不看到链路边和交接内容。

---

## 与 Review / Settlement 的关系

Review Agent 通过买家 Agent 或目标 Agent 的私有接口获取被审查内容。平台只接收 Review Agent 的 verdict：

```text
Passed / Failed / InvalidFormat / ArtifactUnavailable / HashMismatch
```

结算依据：

- Execute assignment 已完成。
- 绑定的 Review assignment 已提交 verdict。
- verdict 满足 SettlementGateway 规则。
- 对应 hold 仍处于 active。

平台不依赖 handoff received / delivered 之类状态结算，因为这些状态会暴露私有链路。

---

## 失败处理

| 失败 | 责任方 |
|------|--------|
| 上游未提供内容 | 下游 / Review Agent 向买家 Agent 或 review 流程报告失败 |
| 下游无法连接上游 | 买家 Agent 重新协调、重试或换人 |
| 下游拒收格式 | Review Agent 提交 `InvalidFormat` 或买家 Agent 自己重排 |
| 上游下线 | 平台 heartbeat timeout，取消未完成 assignment，触发退款 |
| Review Agent 判定 unavailable | 平台记录 verdict，结算网关按规则处理 |

平台处理的是 assignment 存活、审查结论和资金状态，不处理 handoff 过程。
