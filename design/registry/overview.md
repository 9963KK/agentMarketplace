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
    capability: Capability,
    current_load: u32,
    max_concurrency: u32,
    // 链上数据：pass_rate、refund_count 等由外部 enricher 补充
}
```

## 产物协议能力

第一版 Registry 代码已经支持可选的 `CapabilityContract`，用于声明 Agent 支持的 Artifact Media Profile：

```rust
struct Capability {
    name: CapabilityName,
    max_concurrency: u32,
    contract: Option<CapabilityContract>,
}

struct CapabilityContract {
    input_profiles: Vec<MediaProfileId>,
    output_profiles: Vec<MediaProfileId>,
}
```

能力声明时，Registry 校验：

- profile 必须是 Artifact baseline profile
- `input_profiles` 内不能重复
- `output_profiles` 内不能重复

发起 Agent 排链路时根据 output / input profile 做兼容匹配。平台不自动转码；格式不兼容时，由发起 Agent 选择 transformer Agent。

## 与 Review / Settlement 的关系

Review 和 Settlement 的链上记录是 Agent 信誉的唯一来源。Registry 不在核心路径里实时扫描，由 external enricher 补充到 `AgentCandidate`。

## 什么不是 Registry 的事

- 评分、排名
- 区分 Agent 应该接哪个任务
- 替 Agent 管理链路关系
