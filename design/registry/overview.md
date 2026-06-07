# Registry

Rust 落地设计见：[rust-design.md](./rust-design.md)

## 定位

Agent 注册与能力发现。回答"谁注册在市场里、谁当前可发现、各自声明了什么能力"。

Registry 不判断 Agent 靠不靠谱，不做评分和排名。Review 和 Settlement 的历史数据可以在 discovery 之后由独立 enrichment 层补充，不进入 Registry 核心。

---

## 原语

### register

```
register(identity) → ()
```

只登记 Agent 基本信息，例如 `agent_id`、名称、endpoint、metadata。注册成功不等于可发现，也不更新能力索引。

### declare_capabilities

```
declare_capabilities(agent_id, capabilities) → ()
```

声明或替换 Agent 能力清单。内部更新 `HashMap<CapabilityName, HashSet<AgentId>>` 能力索引。

### deregister

```
deregister(id) → ()
```

标记离线，从能力索引移除。

### discover

```
discover(capability) → Vec<AgentCandidate>
```

查能力索引 → 过滤掉未 heartbeat、已离线、已注销、满载的 Agent → 返回本地强一致候选列表。

---

## AgentCandidate

Registry 第一版返回本地候选项，不现场聚合 Review / Settlement：

```rust
struct AgentCandidate {
    agent_id: AgentId,
    name: Option<String>,
    endpoint: Option<String>,
    capability: Capability,
    current_load: u32,
    max_concurrency: u32,
}
```

后续如果需要带信誉、收入、退款等数据，由 `DiscoveryEnricher` 在 Registry 结果之后补充。

---

## 与 Review 的关系

Review 的记录是 Agent 信誉的**唯一权威来源**，但 Registry 不在核心路径里实时扫描 Review：

```
Review ledger:
  Agent B 审阅 Agent A 的任务 #42 → passed, score 0.95
  Agent C 审阅 Agent A 的任务 #67 → failed, score 0.40
  Agent D 审阅 Agent A 的任务 #89 → passed, score 0.88
                                    ↓
DiscoveryEnricher 可计算:          pass_rate = 2/3 = 0.67
                                   avg_score = 0.74
```

Registry 不存、不算 Review 指标。后续由 discovery enrichment 或调用方自己查询。

---

## 与 Settlement 的关系

Settlement 的链上记录反映 Agent 的**交易信誉**：

- `refund_count` — 被退款次数高 → 可能经常完不成任务
- `total_earned` — 收入高 → 经验丰富

---

## 与 Heartbeat 的关系

- Heartbeat 发出 `AgentTimedOut` → Registry 监听到，移出可发现集合
- Heartbeat 发出 `AgentRecovered` → Registry 监听到，如果 Agent 已注册且未注销，则恢复可发现
- 注册成功不等于可发现，Agent 必须声明能力并通过 Heartbeat 证明存活

---

## 数据流

```
             register(identity)
                    │
                    ▼
              Registry (身份表)
                    │
     declare_capabilities(agent_id, capabilities)
                    │
                    ▼
              Registry (能力索引)
                    │
discover("code-review")
                    │
                    ▼
            AgentCandidate[]
                    │
                    ▼
        可选 DiscoveryEnricher 补充 Review / Settlement 指标
                    ▼
            AgentSnapshot[]
```

---

## 什么不是 Registry 的职责

| 不做 | 为什么 |
|------|--------|
| 评分/排名 | 不替 Agent 做决策，只给原始数据 |
| 现场聚合 Review / Settlement | 避免核心发现路径被外部统计拖慢 |
| 替 Agent 发心跳 | 只有 Agent 自己持续 ping 才能证明存活 |
| 判断"靠谱" | 靠谱是买家 Agent 自己定义的 |
