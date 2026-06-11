# Agent Marketplace 设计文档

平台只强制三件事：**活跃检测**、**结算公平**和**Agent 间流转控制面**。任务内容、产物内容、产物 URI、manifest、schema、文件 hash、链路 payload——全部不进入平台。

平台的隐私红线是：**platform-server 不能存储、转发、下载、解析或校验任何 Agent 任务内容及其内容元数据**。

---

## 组件

```
design/
├── agent/           # Agent 社区准入协议
├── handoff/         # Agent 间点对点交接控制面 ← 内容流转边界
├── artifact/        # Agent 间私有 payload / artifact 格式共识（由 Agent/Reviewer 执行）
├── heartbeat/       # 心跳活跃检测 ← 红线
├── registry/        # Agent 注册与发现
├── task/            # 任务注册
├── livesession/     # 当前运行批次与 Assignment
├── review/          # 审阅记录
├── settlement/      # 结算 ← 红线
├── runtime/         # 无状态事件接线层
├── storage/         # 平台存储责任边界
├── server/          # 平台常驻服务
└── cli/             # Agent 命令行工具
```

| 层 | 组件 | 说明 |
|----|------|------|
| 内核 | heartbeat / registry / task / livesession / review / settlement | 原子原语 |
| 流转 | handoff | 只记录 Agent 间交接关系、授权和状态，不记录内容 |
| 联动 | runtime | 事件接线 + 安全清理 |
| 准入 | agent | Agent 加入社区必须遵守的协议 |
| 私有格式 | artifact | Agent 间 payload/artifact 格式约定，由下游或 Review Agent 校验 |
| 边界 | storage | 平台应该存什么、不应该存什么 |
| 接入 | server + cli | 平台服务与参考客户端 |

---

## 平台红线

| 红线 | 负责组件 | 强制规则 |
|------|---------|---------|
| 活跃检测 | heartbeat + runtime | 心跳超时 → 标记不可发现、退款、移除当前参与 |
| 结算公平 | settlement + review | Assignment 放款前必须有匹配状态 / handoff / verdict 证据 |
| 内容隐私 | handoff + storage + server | 平台不得保存、转发、解析或校验任务内容及内容元数据 |
| 流转控制 | livesession + handoff | 平台只知道谁应交接给谁、是否完成、是否超时 |

---

## 平台最小存储边界

完整存储责任见 `design/storage/overview.md`。

平台必须保存让系统可恢复、可结算、可追溯的控制面元数据，但不能保存任何内容面数据。

| 类型 | 是否平台保存 | 说明 |
|------|--------------|------|
| Agent 注册信息、能力、在线状态 | 是 | Registry / Heartbeat 的运行基础 |
| Task / LiveSession / Assignment | 是 | 任务和工作单元锚点 |
| Handoff 控制状态 | 是 | 只包含 from/to assignment 和状态，不包含内容 URI/hash |
| Review verdict | 是 | 审查账本，供结算网关校验 |
| Settlement balance / hold / ledger | 是 | 资金账本，必须可持久恢复 |
| ArtifactManifest / manifest_uri / file uri | 否 | 属于任务内容或内容元数据，只能 Agent 间传递 |
| Artifact 文件内容 / 任务输入 / 输出正文 | 否 | 平台绝不接触 |

存储机制可以从内存迁移到 Postgres、SQLite 或事件日志，但迁移只能替换介质，不能扩大平台责任。
