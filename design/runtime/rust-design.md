# Runtime Rust Design

## 目标

Runtime 是平台组件接线层。它消费 Heartbeat 事件，并把事件转换成固定的安全清理动作。

它回答两个问题：

- Agent 掉线时，哪些平台状态必须立即变得安全
- Agent 恢复时，哪些平台状态可以恢复可发现性

Runtime 不保存业务状态，不编排任务链路，不选择 Agent，不判断 Review，不主动 release。

## 设计选择

Runtime 不采用 `Core + Service` 状态机结构，因为它没有自己的持久状态。第一版采用：

```text
Runtime              组件 handle 聚合器
RuntimeHeartbeatSink HeartbeatEventSink 适配器
RuntimeClock         sink 使用的时间源
RuntimeEventReport   单次事件处理报告
```

直接调用时，调用方使用 `handle_heartbeat_event_at(event, at)`，时间戳显式传入。

接入 `HeartbeatService` 时，`RuntimeHeartbeatSink` 从 `RuntimeClock` 获取时间戳。

## 模块结构

```text
src/
├── runtime/
│   ├── mod.rs
│   ├── core.rs       # Runtime 和 HeartbeatEventSink adapter
│   ├── clock.rs      # RuntimeClock、SystemRuntimeClock、FixedRuntimeClock
│   └── types.rs      # RuntimeEventReport、RuntimeAction、错误报告
├── heartbeat/
├── registry/
├── livesession/
├── settlement/
└── task/
```

## 数据模型

```rust
pub struct Runtime {
    registry: RegistryHandle,
    settlement: SettlementHandle,
    live_sessions: LiveSessionHandle,
    tasks: TaskHandle,
}

pub struct RuntimeHeartbeatSink<C = SystemRuntimeClock> {
    runtime: Runtime,
    clock: C,
}
```

Runtime 只持有各组件 handle。handle 本身是 Tokio 命令发送端，Runtime 不直接持有组件内部状态。

## 时间源

```rust
pub trait RuntimeClock: Clone + Send + Sync + 'static {
    fn now(&self) -> Timestamp;
}

pub struct SystemRuntimeClock;
pub struct FixedRuntimeClock;
```

`SystemRuntimeClock` 使用 UNIX 毫秒时间戳。`FixedRuntimeClock` 用于测试 sink 行为，避免测试依赖真实时间戳。

Runtime 的核心 API 仍然要求显式传入 `Timestamp`，这样直接单元测试不依赖系统时间。

## 核心原语

```rust
impl Runtime {
    pub fn new(
        registry: RegistryHandle,
        settlement: SettlementHandle,
        live_sessions: LiveSessionHandle,
        tasks: TaskHandle,
    ) -> Self;

    pub fn heartbeat_sink(&self) -> RuntimeHeartbeatSink<SystemRuntimeClock>;

    pub fn heartbeat_sink_with_clock<C>(&self, clock: C) -> RuntimeHeartbeatSink<C>
    where
        C: RuntimeClock;

    pub async fn handle_heartbeat_event_at(
        &self,
        event: HeartbeatEvent,
        at: Timestamp,
    ) -> RuntimeEventReport;
}
```

`RuntimeHeartbeatSink` 实现 `HeartbeatEventSink`：

```rust
impl<C> HeartbeatEventSink for RuntimeHeartbeatSink<C>
where
    C: RuntimeClock,
{
    fn publish(
        &self,
        event: HeartbeatEvent,
    ) -> impl Future<Output = Result<(), PublishError>> + Send;
}
```

sink 内部调用 `Runtime.handle_heartbeat_event_at(event, clock.now())`。如果清理报告里有错误，第一版只写日志，不让 Heartbeat 服务停止。

## AgentTimedOut 流程

```text
HeartbeatEvent::AgentTimedOut { agent_id }
  -> Registry.mark_timed_out(agent_id)
  -> Settlement.active_holds_for_agent(agent_id)
  -> filter hold.agent_id == agent_id
  -> Settlement.refund(hold_id)
  -> LiveSession.assignments_by_agent(agent_id)
  -> filter assignment.status == Assigned
  -> LiveSession.cancel_assignment(assignment_id)
  -> Task.active_tasks_by_agent(agent_id)
  -> Task.remove_participant(task_id, agent_id)
```

语义：

- Registry 只标记不可发现，不注销 Agent
- Settlement 只退款绑定到掉线 Agent 工作单元的 Active hold
- Agent 作为付款方的 hold 不会因为付款方掉线被自动退款
- LiveSession 只取消 `Assigned` 状态，保留已 `Submitted` 输出
- Task 只从当前参与者集合移除 Agent，不删除历史参与记录
- Runtime 不区分 executor / reviewer，只看 Agent 是否承担了 Assignment

## AgentRecovered 流程

```text
HeartbeatEvent::AgentRecovered { agent_id }
  -> Registry.mark_alive(agent_id)
```

恢复只影响 Registry 可发现性。

Runtime 不自动：

- 把 Agent 加回 Task
- 恢复已取消 Assignment
- 恢复已退款 hold
- 重新创建 Review
- 触发 release

## 事件报告

```rust
pub struct RuntimeEventReport {
    pub event: HeartbeatEvent,
    pub at: Timestamp,
    pub actions: Vec<RuntimeAction>,
    pub errors: Vec<RuntimeActionError>,
}

pub enum RuntimeAction {
    RegistryMarkedTimedOut { agent_id: AgentId },
    RegistryMarkedAlive { agent_id: AgentId },
    HoldRefunded { hold_id: HoldId },
    AssignmentCancelled { assignment_id: AssignmentId },
    TaskParticipantRemoved { task_id: TaskId, agent_id: AgentId },
}

pub struct RuntimeActionError {
    pub kind: RuntimeActionKind,
    pub target: String,
    pub message: String,
}
```

报告用于测试和后续观测。第一版不做持久化 outbox。

## 错误处理

Runtime 的错误处理原则是：单个动作失败不阻塞其他动作。

```text
refund(hold-1) failed
  -> report.errors.push(...)
  -> continue refund(hold-2)
  -> continue cancel_assignment(...)
  -> continue remove_participant(...)
```

错误分类：

```rust
pub enum RuntimeActionKind {
    MarkRegistryTimedOut,
    MarkRegistryAlive,
    ListActiveHoldsForAgent,
    RefundHold,
    ListAssignmentsByAgent,
    CancelAssignment,
    ListActiveTasksByAgent,
    RemoveTaskParticipant,
}
```

`RuntimeHeartbeatSink.publish()` 始终返回 `Ok(())`，因为 Runtime 清理失败不应该让 Heartbeat 服务停止扫描。失败细节进入 `RuntimeEventReport`，sink 第一版只打印日志。

## 不变量

- Runtime 不持有业务状态
- Runtime 不调用 `deposit()`
- Runtime 不调用 `release()`
- Runtime 不创建 Task / LiveSession / Assignment / Review
- Runtime 不读取 Review verdict
- Runtime 不判断任务阶段
- timeout 清理只处理安全释放：不可发现、退款、取消未提交工作、移除当前参与
- recovery 只恢复可发现性
- 每个清理动作失败都要记录，但不能中断后续动作

## 与其他组件的关系

```text
Heartbeat -> Runtime -> Registry
                    -> Settlement
                    -> LiveSession
                    -> Task
```

Runtime 只消费 Heartbeat 事件。它不消费 Review 事件，不读取 Settlement ledger 做策略判断。

发布者 Agent 仍然负责业务流程：

```text
Registry.discover()
Task.add_participant()
LiveSession.assign()
Settlement.hold()
Review.request()
Review.collect_by_assignment()
Settlement.release()
Task.complete()
```

## 测试策略

- timeout event 标记 registry 不可发现
- timeout event refund 绑定到掉线 Agent 的 active hold
- timeout event 不 refund 仅作为付款方相关的 hold
- timeout event cancel `Assigned` assignment
- timeout event 不 cancel `Submitted` assignment
- timeout event 从 active task participants 移除 Agent
- timeout event 保留 task participant history
- recovered event 标记 registry alive 并恢复 discovery
- RuntimeHeartbeatSink 能接入 HeartbeatService 并转发 timeout event
- 清理动作失败时记录 error 并继续后续动作

## 暂不做

- Runtime service 状态机
- 自动选 Agent
- 自动创建 Assignment
- 自动创建 Review
- 自动 release
- 任务阶段状态机
- retry / outbox / dead letter
- 分布式协调

第一版 Runtime 只做无状态接线和掉线安全清理。
