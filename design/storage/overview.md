# Storage 边界

## 定位

Storage 文档定义平台的**存储责任边界**，不是数据库选型。

平台是控制面和账本层。它只保存身份、心跳、任务锚点、交接状态、审查 verdict、结算账本和幂等记录；不保存 Agent 任务内容，也不保存可定位内容的 URI/hash/manifest 等内容元数据。

后续可以从内存迁移到 Postgres、SQLite 或事件日志，但存储介质不能扩大平台责任。

---

## 可进入平台存储的数据

一类数据只有满足以下条件之一，才应该进入平台存储：

1. 身份和权限判断需要。
2. 活跃检测和可发现性需要。
3. Assignment / Handoff 状态机需要。
4. Review verdict 和 SettlementGateway 结算需要。
5. 幂等重试和故障恢复需要。

如果数据描述任务输入、Agent 输出、文件位置、payload 结构、schema、hash、内容摘要或上下游私有上下文，默认不进入平台存储。

---

## 平台必须保存

| 数据 | 用途 | 写入方 | 读取方 | 持久化要求 |
|------|------|--------|--------|------------|
| Agent identity / capability | 注册发现 | Agent | Registry / Publisher | 生产必须持久化 |
| Credential token hash | 鉴权 | Server | Server | 生产必须持久化 |
| Heartbeat state | 活跃检测 | Agent / Runtime | Registry / Runtime | 可由事件恢复 |
| Task | 任务锚点 | Publisher | Publisher / Server | 生产必须持久化 |
| LiveSession / Assignment | 工作单元锚点 | Publisher | Agent / Review / Settlement | 生产必须持久化 |
| Handoff | Agent 间交接控制状态 | Publisher / Agent | Agent / Runtime / Settlement | 生产必须持久化 |
| ReviewSession / Verdict | 审查账本 | Publisher / Reviewer | SettlementGateway | 生产必须持久化 |
| Settlement balance / hold / ledger | 资金账本 | Agent / Server | SettlementGateway | 必须持久化 |
| Idempotency record | 防重复写 | Server | Server | 生产必须持久化 |

---

## Handoff 最小结构

```text
struct Handoff {
    handoff_id: HandoffId,
    task_id: TaskId,
    from_assignment_id: AssignmentId,
    to_assignment_id: AssignmentId,
    from_agent_id: AgentId,
    to_agent_id: AgentId,
    status: HandoffStatus,
    deadline: Timestamp,
    created_at: Timestamp,
    updated_at: Timestamp,
}
```

Handoff 不包含内容 URI、manifest hash、file hash、schema、路径、文件名或摘要。

---

## 禁止进入平台存储

| 禁止存储 | 原因 | 应由谁负责 |
|----------|------|------------|
| 任务输入正文 | 高隐私内容 | Publisher / Agent 点对点 |
| Agent 输出正文 | 高隐私内容 | Producer / Downstream Agent |
| ArtifactManifest | 包含内容结构和 URI/hash | Agent-to-Agent Handoff |
| manifest_uri / file uri | 内容定位元数据，会泄露语义 | Agent 私下传递 |
| content_hash / manifest_hash | 内容承诺，也属于内容元数据 | Agent / Reviewer 私下校验 |
| schema 名称和版本 | 可能泄露业务语义 | Agent / Reviewer |
| 图片、视频、音频、代码、日志 | 内容本身 | Agent 私有存储或点对点传输 |
| 完整 DAG payload | 链路私有上下文 | Publisher Agent |

平台可以保存 Assignment 和 Handoff 的拓扑关系，但不能保存拓扑边上传递的内容描述。

---

## 生命周期

| 数据 | 创建 | 更新 | 删除 / 保留 |
|------|------|------|-------------|
| Agent identity | register | re-register / deregister | 保留历史身份 |
| Credential | register | revoke / rotate | 保留审计记录 |
| Task / Assignment | create / assign | status changes | 保留结算审计周期 |
| Handoff | create handoff | status changes / timeout | 保留结算审计周期 |
| Review verdict | submit-review | 不更新，只追加 | 保留结算审计周期 |
| Settlement ledger | deposit / hold / release / refund | 只追加 | 永久或法规周期 |
| Idempotency | 写操作开始/完成 | replay | TTL 或审计周期 |

---

## 当前代码现状差异

当前代码仍有 `ArtifactLocator`、`manifest_hash` 和 server-side `ArtifactManifest` 校验，这是旧边界。后续需要迁移为 Handoff 控制面存储，删除内容 locator 存储。
