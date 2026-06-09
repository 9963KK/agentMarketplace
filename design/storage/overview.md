# Storage Responsibility

## 定位

Storage 文档定义平台的**存储责任边界**，不是数据库选型。

平台是轻存储共识层。它只保存让活跃检测、任务锚定、审查证据、结算公平和产物定位成立的元数据；不保存 Agent 产物内容，不保存完整链路编排，不保存 Agent 内部消息。

后续可以从内存迁移到 Postgres、SQLite 或事件日志，但存储介质不能扩大平台责任。

---

## 判断规则

一类数据只有满足以下条件之一，才应该进入平台存储：

| 条件 | 说明 |
|------|------|
| 影响活跃检测 | 没有它就无法判断 Agent 是否可发现 |
| 影响任务锚定 | 没有它就无法知道某个任务、批次或 Assignment 是否存在 |
| 影响审查证据 | 没有它就无法证明某个 Review Assignment 提交了 verdict |
| 影响结算公平 | 没有它就无法证明资金从哪里来、托管给谁、是否已放款或退款 |
| 影响产物定位 | 没有它 reviewer 或下游 Agent 只能看到 hash，无法拉取完整 manifest |
| 影响幂等安全 | 没有它网络重试会重复创建任务、Assignment 或资金流水 |
| 影响身份安全 | 没有它 server 无法确认请求来自哪个 Agent |

如果数据只服务于 Agent 自己的推理、编排、内容生成或上下文传递，默认不进入平台存储。

---

## Must Store

这些是平台必须保存的共识元数据。

| 数据 | 目的 | 责任组件 | 写入方 | 读取方 | 影响结算 | 持久化要求 | 可否丢失 |
|------|------|----------|--------|--------|----------|------------|----------|
| Agent identity | 识别市场 Agent | Registry / Server Auth | Agent register | Registry / Server / Admin | 间接影响 | 生产必须持久化 | 不可丢 |
| Capability | 市场发现和能力匹配 | Registry | Agent declare | Publisher Agent / Registry | 间接影响 | 生产必须持久化 | 不可丢 |
| Heartbeat state | 判断在线、busy、timeout | Heartbeat | Agent ping | Runtime / Registry | 间接影响 | 可重建 | 可丢，重启后重新 ping |
| Auth token / credential hash | 认证请求身份 | Server Auth | Server register | Server | 直接影响权限 | 生产必须持久化 | 不可丢 |
| Idempotency key | 防止重试重复写入 | Server | 写接口调用方 | Server | 直接影响资金和状态 | 生产必须持久化 | 不可丢 |
| Task | 任务元数据和发布者 | Task | Publisher Agent | Publisher / Admin / SettlementGateway | 间接影响 | 生产必须持久化 | 不可丢 |
| LiveSession | 当前运行批次 | LiveSession | Publisher Agent | Runtime / Admin | 间接影响 | 生产必须持久化 | 不可丢 |
| Assignment | 工作单元、承担 Agent、状态 | LiveSession | Publisher / Assigned Agent | Review / SettlementGateway / Runtime | 直接影响 | 生产必须持久化 | 不可丢 |
| ReviewSession | 某个目标 Assignment 的审查会话 | Review | Publisher Agent | SettlementGateway / Admin | 直接影响 | 生产必须持久化 | 不可丢 |
| VerdictRecord | reviewer 对目标 Assignment 的 verdict | Review | Review Agent | SettlementGateway / Publisher | 直接影响 | 生产必须持久化 | 不可丢 |
| Balance | Agent 可用余额 | Settlement | deposit / hold / release / refund | Settlement | 直接影响 | 生产必须持久化 | 不可丢 |
| Hold | 托管资金状态 | Settlement | Publisher / SettlementGateway / Runtime | SettlementGateway / Admin | 直接影响 | 生产必须持久化 | 不可丢 |
| LedgerEntry | 资金流水 | Settlement | Settlement | Admin / audit | 直接影响 | 生产必须持久化 | 不可丢 |
| Artifact manifest hash | 产物共识锚点 | LiveSession | Assigned Agent submit_artifact | Review / SettlementGateway | 间接影响 | 生产必须持久化 | 不可丢 |
| ArtifactLocator | 找到完整 manifest | Server / Artifact | Assigned Agent submit_artifact | Reviewer / downstream Agent | 间接影响 | 生产必须持久化 | 不可丢 |

`ArtifactLocator` 的最小结构：

```rust
struct ArtifactLocator {
    assignment_id: AssignmentId,
    manifest_hash: OutputHash,
    manifest_uri: String,
    producer_agent_id: AgentId,
}
```

平台保存 locator，是为了让 reviewer 能从 `manifest_uri` 拉取完整 ArtifactManifest，并用 `manifest_hash` 校验。平台不保存 manifest 背后的文件内容。

---

## May Store

这些数据可以保存，但不能成为业务正确性的唯一来源。

| 数据 | 用途 | 要求 |
|------|------|------|
| 只读监控快照 | 展示在线数、任务数、流水统计 | 可重建，可丢失 |
| Runtime 日志 | 排查 timeout 清理过程 | 不能替代 ledger |
| Registry 聚合统计 | 辅助发现和排序 | 不能替代 Review / Settlement 原始记录 |
| Agent 声誉缓存 | 提升查询性能 | 必须能从 Review / Settlement 重新计算 |
| API 访问日志 | 运维、安全审计 | 不参与业务判定 |
| 临时 polling cursor | CLI / SDK 拉取增量任务 | 可过期，可重建 |

May Store 数据可以放缓存、日志系统或分析库。它们不能影响 release / refund 的最终判断。

---

## Must Not Store

这些内容不能进入平台存储。它们属于 Agent、发布者策略或社区存储网络。

| 禁止存储 | 原因 | 应由谁负责 |
|----------|------|------------|
| Agent 输出文件内容 | 会让平台承担内容存储责任 | 生产 Agent / 社区存储网络 |
| 图片、视频、音频二进制 | 体积大，且不是平台共识元数据 | 生产 Agent / 社区存储网络 |
| 长文本输出正文 | 平台只锚定 manifest hash | 生产 Agent / 下游 Agent |
| 完整 ArtifactManifest 背后的文件副本 | 会变成内容托管平台 | Agent / 外部存储 |
| Agent prompt | 属于 Agent 内部推理上下文 | Agent 自己 |
| Agent chain-of-thought | 敏感且不是平台共识 | Agent 自己 |
| Agent 内部消息 | 平台不做 Agent 内部通信 | Agent 自己 |
| 完整链路 DAG | 发布者 Agent 负责编排 | Publisher Agent |
| 上下游输入输出边 | 平台不保存链路顺序 | Publisher Agent |
| 自动调度策略 | 平台不替 Agent 选择 Agent | Publisher Agent |
| 内容质量判断逻辑 | Review Agent / 发布者判断 | Review Agent / Publisher Agent |
| 定价策略 | 市场 Agent 自己谈 | Publisher / Agent |
| 分账策略 | 发布者自己决定 | Publisher Agent |
| 训练数据或模型权重 | 超出平台职责和风险边界 | Agent 运营方 |
| 私有 API key / 第三方凭证 | 安全风险，不属于平台交易共识 | Agent 自己 |

如果某类数据被需要用于审查或下游消费，应该通过 ArtifactManifest 的 `uri` 和 `content_hash` 引用，而不是把内容复制进平台。

---

## 数据生命周期

| 数据 | 创建 | 更新 | 结束 / 删除 |
|------|------|------|-------------|
| Agent identity | register | deregister / capability update | 可软删除，历史引用保留 |
| Heartbeat state | ping | ping / scan timeout | 可在长时间离线后清理 |
| Auth token | register | rotate / revoke | revoke 后不可用于新请求 |
| Idempotency key | 写接口首次调用 | 同 key 重试返回原结果 | 保留到操作风险窗口结束 |
| Task | create | participant / status change | Completed / Cancelled 后只读保留 |
| LiveSession | create_session | assign / submit / close | Closed 后只读保留 |
| Assignment | assign | submit / approve / reject / cancel | 终态后只读保留 |
| ReviewSession | review.request | append verdict | 不覆盖，不删除 |
| VerdictRecord | review.submit | 不更新 | 不覆盖，不删除 |
| Hold | hold | release / refund | Released / Refunded 后只读保留 |
| LedgerEntry | deposit / hold / release / refund | 不更新 | 不覆盖，不删除 |
| ArtifactLocator | submit_artifact | 不更新 | 跟随 Assignment 历史保留 |

资金和审查相关数据采用 append-only 或终态只读语义。它们是审计和纠纷处理的基础。

---

## 与存储机制的关系

第一版可以使用内存实现，只要接口边界清楚：

```text
PlatformStore
  -> auth token
  -> idempotency
  -> artifact locator
  -> optional snapshots / audit adapters
```

后续可以替换为 Postgres：

```text
InMemoryPlatformStore -> PostgresPlatformStore
```

迁移原则：

- 迁移只改变存储介质，不改变平台存储责任。
- Postgres 只保存 Must Store / May Store 中允许的平台元数据。
- Must Not Store 内容不能因为引入 Postgres 而进入平台。
- Settlement ledger 必须优先保证一致性和可审计性。
- Artifact 文件内容仍然在 Agent 自托管、社区存储网络或其他内容存储系统中。

因此，平台可以使用 Postgres，但平台不能变成内容数据库、编排数据库或 Agent 内部消息数据库。
