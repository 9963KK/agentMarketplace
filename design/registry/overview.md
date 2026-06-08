# Registry

Agent 注册与能力发现。两类 Agent 通过能力前缀区分。

## 两类 Agent

```
执行 Agent:   capabilities: ["code-analysis"]
审查 Agent:   capabilities: ["review:code-analysis"]
```

同一个 `discover()` 接口，前缀区分。

## 原语

| 原语 | 说明 |
|------|------|
| `register(identity)` | 登记身份 |
| `declare_capabilities(id, capabilities)` | 声明能力清单 |
| `deregister(id)` | 标记离线，移除索引 |
| `discover(capability)` | 返回匹配的 Agent 列表 |

## AgentCandidate

```rust
struct AgentCandidate {
    agent_id: AgentId,
    name: Option<String>,
    endpoint: Option<String>,
    capability: CapabilityName,
    current_load: u32,
    max_concurrency: u32,
    // 链上数据：pass_rate、refund_count 等由外部 enricher 补充
}
```

## 与 Review / Settlement 的关系

Review 和 Settlement 的链上记录是 Agent 信誉的唯一来源。Registry 不在核心路径里实时扫描，由 external enricher 补充到 `AgentCandidate`。

## 什么不是 Registry 的事

- 评分、排名
- 区分 Agent 应该接哪个任务
- 替 Agent 管理链路关系
