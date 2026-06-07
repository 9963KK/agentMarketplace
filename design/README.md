# Agent Marketplace 设计文档

两层划分：[平台红线](#平台红线) vs [Agent 自由](#agent-职责)。

平台不承担 Agent 内部通信。Agent 之间通过 `discover()` 拿到元数据后自行通信。

---

## 平台红线

| 红线 | 负责组件 | 说明 |
|------|---------|------|
| 活跃检测 | heartbeat | 心跳超时 → 强制释放任务 |
| 结算公平 | settlement | 放款凭 review + chain 记录，掉线自动退款 |

---

## 组件（原子化）

```
design/
├── heartbeat/           # 心跳与活跃检测 ← 红线
├── registry/            # Agent 注册与发现
├── chain/               # 任务链路账本
├── review/              # 审阅会话与 verdict 记录
└── settlement/          # 结算 ← 红线
```

每个组件独立，只做一件事。

| 组件 | 回答的问题 | 平台存什么 | 平台不存什么 |
|------|-----------|-----------|-------------|
| heartbeat | Agent 还活着吗 | 心跳时间戳 | — |
| registry | 谁能做什么、靠不靠谱 | 能力索引、链上元数据快照 | 排名、评分逻辑 |
| chain | 任务从谁流向谁、产出了什么、谁审查 | 节点关系、executor、reviewers、artifact hash、holder 承诺 | artifact 正文 |
| review | 审阅结果是什么 | review session 快照、verdict、审阅者、时间戳 | 被审阅内容 |
| settlement | 钱在哪、该给谁 | 余额、托管记录、流水 | 定价逻辑 |

---

## Agent 职责

Agent 自己负责，平台不介入：

- 匹配 — `discover()` 拿到列表和元数据后自己选
- 通信 — Agent 之间直连，平台不传话
- 协商 — 自己谈价格、时间
- 判定 — 收集 verdict 后自己决定过没过
- 编排 — 返工几次、什么时候放弃
- 定价 — 任务值多少、审阅费多少
