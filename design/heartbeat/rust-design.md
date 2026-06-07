# Heartbeat Rust Design

## 目标

Heartbeat 是平台活跃检测红线。Rust 版本只解决一个问题：判断 Agent 是否仍然活着，并在超时时发出事件。

它不负责：

- 移除 Registry 索引
- 释放任务
- 自动退款
- 判定 Agent 最终生命周期状态

这些动作由订阅方处理。Heartbeat 的输出只有事实：某个 Agent 心跳超时。

## 设计选择

可选方案：

| 方案 | 说明 | 取舍 |
|------|------|------|
| `Arc<RwLock<HashMap>>` 共享状态 | 调用方直接读写 Heartbeat 状态 | 简单，但并发边界扩散，后续容易被业务逻辑污染 |
| Tokio actor | Heartbeat 独占状态，通过 channel 接收命令 | 并发边界清楚，但纯逻辑测试稍重 |
| 纯核心 + Tokio 服务 | `HeartbeatCore` 管状态，`HeartbeatService` 管定时器和消息 | 推荐。核心可确定性测试，服务层只处理 IO |

采用 **纯核心 + Tokio 服务**。

## 模块结构

```text
src/
├── heartbeat/
│   ├── mod.rs
│   ├── core.rs       # 纯状态机
│   ├── service.rs    # Tokio 定时扫描与命令循环
│   └── types.rs      # AgentId、配置、事件、错误
└── runtime/
    └── ...           # 后续由运行时负责事件接线
```

## 核心原语

```rust
pub struct HeartbeatCore {
    pings: HashMap<AgentId, PingInfo>,
    config: HeartbeatConfig,
}

impl HeartbeatCore {
    pub fn ping(&mut self, agent_id: AgentId, busy: bool, now: Instant) -> PingOutcome;
    pub fn is_alive(&self, agent_id: &AgentId, now: Instant) -> bool;
    pub fn scan(&mut self, now: Instant) -> Vec<HeartbeatEvent>;
    pub fn forget(&mut self, agent_id: &AgentId) -> bool;
}
```

`HeartbeatCore` 不启动线程，不读系统时间，不直接发消息。所有时间都由调用方传入，保证测试可控。

## 数据模型

```rust
pub struct HeartbeatConfig {
    pub scan_interval: Duration, // 默认 5s
    pub idle_timeout: Duration,  // 默认 45s
    pub busy_timeout: Duration,  // 默认 15s
}

pub struct PingInfo {
    pub last_ping: Instant,
    pub busy: bool,
    pub timed_out: bool,
}

pub enum HeartbeatEvent {
    AgentTimedOut { agent_id: AgentId },
    AgentRecovered { agent_id: AgentId },
}

pub enum PingOutcome {
    FirstSeen,
    Updated,
    RecoveredFromTimeout,
}
```

`timed_out = true` 后，`scan()` 不再重复发超时事件。下一次 `ping()` 会把 `timed_out` 清回 `false`，返回 `RecoveredFromTimeout`，服务层同时发布 `AgentRecovered`。

## 超时规则

每个 Agent 的阈值由最近一次 `ping(agent_id, busy)` 决定：

| `busy` | 阈值 |
|--------|------|
| `false` | `idle_timeout`，默认 45 秒 |
| `true` | `busy_timeout`，默认 15 秒 |

判断规则：

```text
now - last_ping > selected_timeout
```

只使用 `>`，不使用 `>=`。这样边界行为明确：刚好到阈值时仍视为存活，超过阈值才超时。

## 服务层

`HeartbeatService` 是运行时适配层：

```rust
pub struct HeartbeatServiceOptions {
    pub command_buffer: usize,       // 默认 128
    pub publish_timeout: Duration,   // 默认 1s
}

pub enum HeartbeatCommand {
    Ping { agent_id: AgentId, busy: bool, reply: oneshot::Sender<PingOutcome> },
    IsAlive { agent_id: AgentId, reply: oneshot::Sender<bool> },
    Forget { agent_id: AgentId, reply: oneshot::Sender<bool> },
    Shutdown { reply: oneshot::Sender<()> },
}

pub trait HeartbeatEventSink: Send + Sync + 'static {
    async fn publish(&self, event: HeartbeatEvent) -> Result<(), PublishError>;
}
```

服务循环：

1. 接收 `HeartbeatCommand`
2. 每 `scan_interval` 调用一次 `core.scan(now)`
3. 将 `AgentTimedOut` / `AgentRecovered` 发布到 `HeartbeatEventSink`
4. 单次事件发布超过 `publish_timeout` 后放弃等待，避免阻塞心跳服务
5. `Shutdown` 时停止定时器并退出，退出后再确认调用方

Heartbeat 不知道 Registry 和 Settlement 的存在，只依赖事件出口。

调用方不直接操作 `HeartbeatCommand`，优先使用 `HeartbeatHandle`：

```rust
handle.ping("agent-1", true).await?;
handle.is_alive("agent-1").await?;
handle.forget("agent-1").await?;
handle.shutdown().await?;
```

## 错误处理

核心层不返回 IO 错误。服务层只处理发布失败：

- `scan_interval` / `idle_timeout` / `busy_timeout` 必须大于 0
- `command_buffer` / `publish_timeout` 必须大于 0
- 事件发布失败必须记录日志
- 事件发布超时必须记录日志
- 不回滚 `timed_out`
- 不重复无限发布同一个超时事件

理由：Heartbeat 的事实状态已经确定。事件可靠性应该由运行时或后续 outbox 机制保证，而不是让 Heartbeat 保持复杂重试状态。

## 测试策略

核心测试优先：

- 第一次 `ping` 返回 `FirstSeen`
- 普通 `ping` 更新 `last_ping` 和 `busy`
- 空闲 Agent 45 秒内 `is_alive = true`
- 空闲 Agent 超过 45 秒后 `scan` 发 `AgentTimedOut`
- 忙碌 Agent 超过 15 秒后 `scan` 发 `AgentTimedOut`
- 同一 Agent 超时后重复 `scan` 不重复发事件
- 超时后再次 `ping` 返回 `RecoveredFromTimeout`
- `forget` 后 `is_alive = false` 且后续不发事件
- 多个 Agent 同时超时时一次扫描发出多个事件
- 未知 Agent 查询 `is_alive = false`
- 非法配置被拒绝
- 空白 AgentId 被拒绝

服务测试后置：

- 定时扫描能发布事件
- 超时恢复后能发布 `AgentRecovered`
- `Shutdown` 能干净退出
- 事件发布失败不会 panic
- 事件发布卡住不会永久阻塞服务

## 暂不做

- 分布式 Heartbeat 聚合
- 多节点一致性
- 持久化心跳状态
- 心跳鉴权
- 事件投递重试策略

这些属于后续平台运行时职责。第一版只做单进程内可靠核心。
