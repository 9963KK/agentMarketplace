# Task Rust Design

## 目标

Task 是任务元数据组件。它只回答四个问题：

- 任务是否存在
- 谁发起了任务
- 当前哪些 Agent 参与任务
- 历史哪些 Agent 参与过任务

Task 不保存链路结构，不保存 artifact，不解释 Agent 角色，不判断任务是否成功。角色语义由 Review 和 Settlement 的局部事实表达。

## 设计选择

采用和 Heartbeat / Registry / Review / Settlement 一致的结构：

```text
TaskCore      纯状态机
TaskService   Tokio 命令循环
```

`TaskCore` 不读系统时间，不访问网络，不调用其他组件。所有时间由调用方传入，保证测试可控。

## 模块结构

```text
src/
├── task/
│   ├── mod.rs
│   ├── core.rs       # 纯状态机
│   ├── service.rs    # Tokio 命令循环
│   └── types.rs      # Task、TaskStatus、错误
├── types.rs          # TaskId、Timestamp
├── heartbeat/
├── registry/
├── review/
└── settlement/
```

## 核心数据模型

```rust
pub struct TaskCore {
    tasks: HashMap<TaskId, Task>,
    active_tasks_by_agent: HashMap<AgentId, HashSet<TaskId>>,
    task_history_by_agent: HashMap<AgentId, HashSet<TaskId>>,
    tasks_by_publisher: HashMap<AgentId, HashSet<TaskId>>,
    next_task: u64,
}

pub struct Task {
    pub task_id: TaskId,
    pub publisher: AgentId,
    pub active_participants: HashSet<AgentId>,
    pub participant_history: HashSet<AgentId>,
    pub status: TaskStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub enum TaskStatus {
    Active,
    Completed,
    Cancelled,
}
```

`tasks` 是真相源。三个索引用于快速查询：

- `active_tasks_by_agent`：当前仍参与的活跃任务
- `task_history_by_agent`：曾经参与过的任务
- `tasks_by_publisher`：某发布者发起过的任务

`participant_history` 只增加，不因换人或退出而删除。

## 核心原语

```rust
impl TaskCore {
    pub fn create(
        &mut self,
        publisher: AgentId,
        created_at: Timestamp,
    ) -> Result<TaskId, TaskError>;

    pub fn add_participant(
        &mut self,
        task_id: &TaskId,
        agent_id: AgentId,
        updated_at: Timestamp,
    ) -> Result<(), TaskError>;

    pub fn remove_participant(
        &mut self,
        task_id: &TaskId,
        agent_id: &AgentId,
        updated_at: Timestamp,
    ) -> Result<bool, TaskError>;

    pub fn complete(
        &mut self,
        task_id: &TaskId,
        completed_at: Timestamp,
    ) -> Result<(), TaskError>;

    pub fn cancel(
        &mut self,
        task_id: &TaskId,
        cancelled_at: Timestamp,
    ) -> Result<(), TaskError>;

    pub fn get(&self, task_id: &TaskId) -> Option<&Task>;
    pub fn active_tasks_by_agent(&self, agent_id: &AgentId) -> Vec<Task>;
    pub fn task_history_by_agent(&self, agent_id: &AgentId) -> Vec<Task>;
    pub fn tasks_by_publisher(&self, agent_id: &AgentId) -> Vec<Task>;
}
```

`remove_participant()` 返回 `bool`：`true` 表示确实从当前参与者集合移除，`false` 表示该 Agent 本来就不在当前参与者集合中。

## 生命周期

```text
create(publisher)
  -> 生成 task_id
  -> status = Active
  -> active_participants = {}
  -> participant_history = {}
  -> 写入 tasks_by_publisher

add_participant(task_id, agent)
  -> 任务必须 Active
  -> 写入 active_participants
  -> 写入 participant_history
  -> 写入 active_tasks_by_agent
  -> 写入 task_history_by_agent

remove_participant(task_id, agent)
  -> 任务必须 Active
  -> 仅从 active_participants 移除
  -> 仅从 active_tasks_by_agent 移除
  -> 不修改 participant_history
  -> 不修改 task_history_by_agent

complete(task_id) / cancel(task_id)
  -> 任务必须 Active
  -> status 改为 Completed / Cancelled
  -> 从所有 active_tasks_by_agent 索引中移除该 task_id
  -> participant_history 和 task_history_by_agent 保留
```

完成或取消后，任务进入只读状态。第一版不支持从 `Completed` / `Cancelled` 恢复为 `Active`。

## 不变量

- `Task.task_id` 唯一
- `Task.publisher` 创建后不可变
- `Task.status = Active` 时才允许修改参与者
- `Completed` / `Cancelled` 任务只读
- `active_participants` 是当前参与者集合
- `participant_history` 是历史参与者集合，只增加不删除
- `active_participants` 中的 Agent 必须也存在于 `participant_history`
- `active_tasks_by_agent` 只包含 `Active` 任务
- `task_history_by_agent` 可以包含 `Completed` / `Cancelled` 任务
- `tasks_by_publisher` 不因任务完成或取消而删除
- 所有写操作先校验，再更新状态和索引，避免部分写入

## 与其他组件的关系

Task 不主动调用其他组件。它只提供 `TaskId` 和参与者视图。谁做了哪份可审查、可结算的工作，由 LiveSession / Assignment 表达。

```text
Task.create()
  -> task_id
  -> LiveSession.create(task_id)
  -> Assignment.assign(task_id, session_id, agent_id, kind)
  -> Review.request(..., assignment_id, ...)
  -> Settlement.hold(..., assignment_id, ...)
```

Review 和 Settlement 第一版不反查 Task 校验存在性。调用方负责保证流程顺序：先创建 Task，再创建 LiveSession / Assignment，最后创建 Review / Settlement 记录。

Heartbeat 掉线后的安全清理由 Runtime 接线层完成：

```text
Heartbeat: AgentTimedOut { agent_id }
  -> Settlement.active_holds_for_agent(agent_id)
  -> Settlement.refund(hold_id)
  -> Task.remove_participant(task_id, agent_id)
```

Registry 可以读取 Task 的历史统计作为发现结果的外部补充，但 RegistryCore 不依赖 TaskCore。Runtime 不读取 Review，不判断任务阶段，也不替发布者 Agent 编排任务流程。

## 错误处理

```rust
pub enum TaskError {
    TaskNotFound(TaskId),
    TaskNotActive { task_id: TaskId, status: TaskStatus },
    TimestampWentBackwards {
        task_id: TaskId,
        current: Timestamp,
        attempted: Timestamp,
    },
}
```

`TaskId` 和 `AgentId` 的空值校验沿用已有公共类型。`TaskCore` 只处理业务状态错误。

`updated_at` 不能小于当前 `Task.updated_at`。这能避免乱序调用把任务时间线写回过去。相等时间戳允许，方便同一批操作使用同一个时间。

## 服务层

```rust
pub enum TaskCommand {
    Create {
        publisher: AgentId,
        created_at: Timestamp,
        reply: oneshot::Sender<Result<TaskId, TaskError>>,
    },
    AddParticipant {
        task_id: TaskId,
        agent_id: AgentId,
        updated_at: Timestamp,
        reply: oneshot::Sender<Result<(), TaskError>>,
    },
    RemoveParticipant {
        task_id: TaskId,
        agent_id: AgentId,
        updated_at: Timestamp,
        reply: oneshot::Sender<Result<bool, TaskError>>,
    },
    Complete {
        task_id: TaskId,
        completed_at: Timestamp,
        reply: oneshot::Sender<Result<(), TaskError>>,
    },
    Cancel {
        task_id: TaskId,
        cancelled_at: Timestamp,
        reply: oneshot::Sender<Result<(), TaskError>>,
    },
    Get {
        task_id: TaskId,
        reply: oneshot::Sender<Option<Task>>,
    },
    ActiveTasksByAgent {
        agent_id: AgentId,
        reply: oneshot::Sender<Vec<Task>>,
    },
    TaskHistoryByAgent {
        agent_id: AgentId,
        reply: oneshot::Sender<Vec<Task>>,
    },
    TasksByPublisher {
        agent_id: AgentId,
        reply: oneshot::Sender<Vec<Task>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}
```

服务层职责：

- 顺序处理命令，避免锁扩散
- 返回 `Task` 克隆快照，不暴露内部可变引用
- `Shutdown` 时退出循环后再确认调用方
- 不消费 Heartbeat / Review / Settlement 事件；掉线清理由 Runtime 接线层处理，业务编排由发布者 Agent 处理

## 查询顺序

返回 `Vec<Task>` 时使用确定性顺序，按 `TaskId` 升序排序。这样测试稳定，也避免调用方因为 HashMap / HashSet 遍历顺序不同看到随机结果。

## 测试策略

核心测试优先：

- `create` 创建 Active 任务并建立 publisher 索引
- `add_participant` 同时写入当前参与者和历史参与者
- 重复 `add_participant` 幂等，不重复污染索引
- `remove_participant` 只移除当前参与者，不删除历史参与者
- 移除不存在的当前参与者返回 `false`
- `active_tasks_by_agent` 只返回当前仍参与的 Active 任务
- `task_history_by_agent` 返回曾参与任务，包括已退出任务
- `complete` 后任务只读，且从 active 索引移除
- `cancel` 后任务只读，且从 active 索引移除
- `Completed` / `Cancelled` 任务拒绝继续修改参与者
- 未知 `task_id` 返回 `TaskNotFound`
- 时间戳倒退返回 `TimestampWentBackwards`
- 多任务查询按 `TaskId` 确定性排序

服务测试后置：

- service 能 create / add / remove / query
- service 能 complete / cancel
- service 透传 `TaskError`
- `Shutdown` 后继续调用返回 `Stopped`

## 暂不做

- 任务链路 DAG
- 节点输入输出关系
- artifact 内容或 URL 存储
- executor / reviewer 角色建模
- 任务成功判定
- 自动读取 Review / Settlement
- 分布式持久化

第一版只做任务元数据账本，给 Review / Settlement 提供稳定的 `task_id` 锚点。
