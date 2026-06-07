# Heartbeat

Rust 落地设计见：[rust-design.md](./rust-design.md)

## 定位

Agent 活跃检测。平台红线之一。

**只管心跳本身**。Agent 离线后释放任务 → 是 Registry 的事。自动退款 → 是 Settlement 的事。Heartbeat 只负责说出"他死了"。

---

## 原语

### ping

```
ping(agent_id) → ()
```

Agent 调用。只做一件事：更新 `last_ping` 时间戳。

---

### is_alive

```
is_alive(agent_id) → bool
```

查询某 Agent 是否存活。外部调用（Registry、Settlement 在操作前先问一句）。

---

## 超时事件

Heartbeat 内部有一个**轻量定时扫描**。不是因为惰性检查不够，而是：

- 执行者离线 → 任务需要尽快释放 → 不能等下次 `discover()` 才触发
- 这是平台红线，需要一定的实时性

扫描间隔：**5 秒**。

每次扫描做的事：
1. 遍历所有 Agent
2. 检查 `now - last_ping > threshold`
3. 超时 → 发出 `timed_out` 事件
4. 对该 Agent 停止后续扫描（等它重新 `ping` 再恢复）

---

## 超时阈值

| Agent 状态 | 阈值 | 理由 |
|-----------|------|------|
| 空闲 / 在线 | 45 秒 | 允许短暂无响应 |
| 忙碌（有任务在身） | 15 秒 | 任务挂在它身上，需要快速反应 |

`ping` 时附带一个可选参数 `busy: bool`，Heartbeat 据此选阈值。

```
ping(agent_id, busy)
```

---

## 数据结构

```rust
struct Heartbeat {
    pings: HashMap<AgentId, PingInfo>,
    scan_interval_ms: u64,
    idle_timeout_ms: u64,    // 45s
    busy_timeout_ms: u64,    // 15s
}

struct PingInfo {
    last_ping: Instant,
    is_busy: bool,
    timed_out: bool,         // 已超时，停止对此 Agent 的后续扫描
}
```

---

## 事件

Heartbeat 发出的事件（通过运行时事件出口投递）：

| 事件 | 含义 | 谁关心 |
|------|------|--------|
| `AgentTimedOut { agent_id }` | Agent 心跳超时 | Registry（移出可发现集合）、Settlement（退款） |
| `AgentRecovered { agent_id }` | 超时 Agent 恢复心跳 | Registry（恢复可发现） |

Heartbeat 不直接调 Registry 和 Settlement。只发事件。运行时负责把事件交给关心的组件。

---

## 什么不是 Heartbeat 的事

| 不做 | 谁做 |
|------|------|
| 移出可发现集合 | Registry 收到 `AgentTimedOut` 事件后自己做 |
| 释放任务 | Agent/Registry 自己处理 |
| 自动退款 | Settlement 收到事件后自己触发 |
| 判断 Agent 是否应该离线 | 只报告超时，离线判定是 Registry 的事 |
