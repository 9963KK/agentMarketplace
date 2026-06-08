# Agent Marketplace 设计文档

平台只强制三件事：**活跃检测**、**结算公平**和**产物协议共识**。链路编排、产物存储、调度决策——全部是发起 Agent 自己的事。

---

## 组件

```
design/
├── artifact/        # Agent 输出协议共识
├── heartbeat/       # 心跳活跃检测 ← 红线
├── registry/        # Agent 注册与发现
├── task/            # 任务注册
├── livesession/     # 当前运行批次与 Assignment
├── review/          # 审阅记录
├── settlement/      # 结算 ← 红线
└── runtime/         # 无状态事件接线层
```

| 组件 | 平台存什么 | 平台不管什么 |
|------|-----------|-------------|
| artifact | manifest / media profile 协议 | 文件内容存储、转码 |
| heartbeat | 心跳时间戳 | — |
| registry | 身份 + 能力索引 | 排名、评分逻辑 |
| task | 谁发起的、当前/历史谁参与 | 链路顺序、artifact 内容 |
| livesession | 当前批次、Assignment、manifest hash | 链路顺序、上下游依赖 |
| review | 哪个 Review Assignment 审了哪个 Assignment | 过没过的最终判定 |
| settlement | 哪个 Assignment 该付谁多少钱 | 定价、分账逻辑 |
| runtime | 不存状态 | 任务编排、阶段状态机 |

---

## 平台红线

| 红线 | 负责组件 | 强制规则 |
|------|---------|---------|
| 活跃检测 | heartbeat + runtime | 心跳超时 → 标记不可发现、退款、移除当前参与 |
| 结算公平 | settlement | Assignment 放款前必须有匹配 evidence |
| 产物协议共识 | artifact + livesession | Agent 输出必须是 ArtifactManifest，平台只锚定 manifest hash |
