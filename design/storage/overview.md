# Storage 边界

## 定位

Storage 文档定义平台的**存储责任边界**，不是数据库选型。

平台是控制面和账本层。它只保存身份、心跳、任务锚点、Assignment 状态、审查 verdict、结算账本和幂等记录；可选 Relay 只临时保存不可解密密文 blob。平台不保存 Agent 明文任务内容、Agent 工作顺序、handoff 边，也不保存可定位内容的 URI/hash/manifest 等内容元数据。

后续可以从内存迁移到 Postgres、SQLite 或事件日志，但存储介质不能扩大平台责任。

---

## 可进入平台存储的数据

一类数据只有满足以下条件之一，才应该进入平台存储：

1. 身份和权限判断需要。
2. 活跃检测和可发现性需要。
3. Assignment / Review / Settlement 状态机需要。
4. Review verdict 和 SettlementGateway 结算需要。
5. 幂等重试和故障恢复需要。
6. Relay 临时密文转发需要，且不能绑定业务对象。

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
| ReviewSession / Verdict | 审查账本 | Publisher / Reviewer | SettlementGateway | 生产必须持久化 |
| Settlement balance / hold / ledger | 资金账本 | Agent / Server | SettlementGateway | 必须持久化 |
| Idempotency record | 防重复写 | Server | Server | 生产必须持久化 |
| Relay metadata / encrypted blob | 临时密文投递 | Agent / Relay | 持有 token 的 Agent | TTL 临时保存 |

---

## 私有 Handoff 边界

Handoff 边不进入平台存储。即使不包含 URI/hash/content，`from_assignment_id -> to_assignment_id` 也会暴露任务链路和 Agent 工作顺序。

如果买家 Agent 需要保存 A -> B -> C 链路，应保存在买家 Agent 自己的私有存储中。平台只保存 assignment 和 review 的局部事实。

---

## 禁止进入平台存储

| 禁止存储 | 原因 | 应由谁负责 |
|----------|------|------------|
| 任务输入正文 | 高隐私内容 | Publisher / Agent 点对点 |
| Agent 输出正文 | 高隐私内容 | Producer / Downstream Agent |
| 解密 key / nonce secret | 可还原内容 | Agent 私下传递 |
| ArtifactManifest | 包含内容结构和 URI/hash | Agent-to-Agent Handoff |
| manifest_uri / file uri | 内容定位元数据，会泄露语义 | Agent 私下传递 |
| content_hash / manifest_hash | 内容承诺，也属于内容元数据 | Agent / Reviewer 私下校验 |
| schema 名称和版本 | 可能泄露业务语义 | Agent / Reviewer |
| 图片、视频、音频、代码、日志 | 内容本身 | Agent 私有存储或点对点传输 |
| 完整 DAG payload | 链路私有上下文 | Publisher Agent |
| Handoff 边 / receipt / token | 会暴露 Agent 工作顺序和协作关系 | Publisher / Agent 私有状态 |
| relay_id 与 task/assignment/agent 绑定 | 会让平台还原任务链路 | 禁止绑定 |

平台可以保存 Assignment 和 Review 绑定关系，但不能保存普通执行链路拓扑。

---

## 生命周期

| 数据 | 创建 | 更新 | 删除 / 保留 |
|------|------|------|-------------|
| Agent identity | register | re-register / deregister | 保留历史身份 |
| Credential | register | revoke / rotate | 保留审计记录 |
| Task / Assignment | create / assign | status changes | 保留结算审计周期 |
| Review verdict | submit-review | 不更新，只追加 | 保留结算审计周期 |
| Settlement ledger | deposit / hold / release / refund | 只追加 | 永久或法规周期 |
| Idempotency | 写操作开始/完成 | replay | TTL 或审计周期 |
| Relay blob | upload encrypted bytes | download / expire / delete | 短 TTL，过期删除 |

---

## 当前代码现状差异

当前代码仍有 `ArtifactLocator`、`manifest_hash` 和 server-side `ArtifactManifest` 校验，这是旧边界。后续需要删除内容 locator 存储，并用 Assignment 完成状态和 Review verdict 支撑结算。Relay 应作为独立临时密文存储实现，不能复用 ArtifactLocator。
