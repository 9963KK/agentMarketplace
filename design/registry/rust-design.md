# Registry Rust Design

## 目标

Registry 负责 Agent 注册与能力发现。它回答三个问题：

- 谁注册在市场里
- 谁当前可发现
- 每个 Agent 声明自己能做什么

注册只负责 Agent 基本信息。能力声明是独立动作。Registry 不负责判断 Agent 是否靠谱，不负责给 Agent 排名，不负责结算，不负责审阅。Review 和 Settlement 的历史数据可以作为 discovery 的外部补充信息，但不进入 Registry 核心状态机。

## 设计选择

可选方案：

| 方案 | 说明 | 取舍 |
|------|------|------|
| 直接共享 `Arc<RwLock<RegistryCore>>` | 调用方直接读写 Registry 状态 | 简单，但状态不变量容易被多个调用方破坏 |
| 单体服务内联 Registry | Registry 与 Heartbeat、Review、Settlement 等组件写在同一个服务里 | 接线少，但组件边界不清 |
| 纯核心 + Tokio 服务 | `RegistryCore` 管状态，`RegistryService` 管命令与事件 | 推荐。便于测试，也和 Heartbeat 模式一致 |

采用 **纯核心 + Tokio 服务**。

## 模块结构

```text
src/
├── registry/
│   ├── mod.rs
│   ├── core.rs       # 纯状态机
│   ├── service.rs    # Tokio 命令循环与事件消费
│   └── types.rs      # Profile、Capability、Snapshot、错误
├── heartbeat/
│   └── ...           # 提供 AgentTimedOut / AgentRecovered
└── runtime/
    └── ...           # 后续负责组件接线
```

## 核心数据模型

```rust
pub struct RegistryCore {
    agents: HashMap<AgentId, AgentInfo>,
    alive_agents: HashSet<AgentId>,
    capability_index: HashMap<CapabilityName, HashSet<AgentId>>,
    load: HashMap<AgentId, LoadInfo>,
}

pub struct AgentInfo {
    pub identity: AgentIdentity,
    pub capabilities: BTreeMap<CapabilityName, Capability>,
    pub lifecycle: AgentLifecycle,
}

pub enum AgentLifecycle {
    Registered,
    Deregistered,
}

pub struct AgentIdentity {
    pub agent_id: AgentId,
    pub name: Option<String>,
    pub endpoint: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

pub struct Capability {
    pub name: CapabilityName,
    pub max_concurrency: u32,
    pub contract: Option<CapabilityContract>,
}

pub struct CapabilityContract {
    pub input_profiles: Vec<MediaProfileId>,
    pub output_profiles: Vec<MediaProfileId>,
}

pub struct LoadInfo {
    pub current: u32,
}
```

`agents` 是注册真相源。`AgentIdentity` 只存身份和基础连接信息。`capabilities` 是 Agent 后续声明的能力集合。

`CapabilityContract` 是可选协议契约，声明 Agent 支持的输入 / 输出 Artifact Media Profile。Registry 保存并发现这些契约，只校验 profile 是否属于 baseline，以及同一列表内是否重复；它不读取文件内容，也不替 Agent 做转码。

`alive_agents` 表示 Heartbeat 当前认为活着的 Agent。`capability_index` 用于快速定位声明了某能力的 Agent。`load` 是 Registry 本地的运行时负载视图。

## 不变量

Registry 必须维护这些不变量：

- `agents` 可以包含已超时 Agent，但不能包含非法 `AgentId`
- `deregistered` Agent 不可发现
- `alive_agents` 只能包含已注册且未 deregister 的 Agent
- `capability_index` 可以存全量注册能力，但 `discover()` 必须过滤 `alive_agents` 和 `deregistered`
- `current_load <= max_concurrency` 才可发现
- `AgentTimedOut` 不删除 Agent，只从可发现集合移除
- `AgentRecovered` 只恢复已注册且未 deregister 的 Agent
- `register()` 只更新基础信息，不更新能力索引
- `declare_capabilities()` 覆盖已有能力集合时，必须先完整校验能力和 contract，再移除旧能力索引并插入新能力索引

## 核心原语

```rust
impl RegistryCore {
    pub fn register(&mut self, identity: AgentIdentity) -> Result<RegisterOutcome, RegistryError>;
    pub fn declare_capabilities(
        &mut self,
        agent_id: &AgentId,
        capabilities: Vec<Capability>,
    ) -> Result<CapabilityUpdateOutcome, RegistryError>;
    pub fn deregister(&mut self, agent_id: &AgentId) -> bool;
    pub fn mark_alive(&mut self, agent_id: &AgentId) -> bool;
    pub fn mark_timed_out(&mut self, agent_id: &AgentId) -> bool;
    pub fn set_load(&mut self, agent_id: &AgentId, current: u32) -> Result<(), RegistryError>;
    pub fn discover(&self, query: DiscoveryQuery) -> Vec<AgentCandidate>;
    pub fn get(&self, agent_id: &AgentId) -> Option<&AgentInfo>;
}
```

`RegistryCore` 不读系统时间，不发消息，不调用 Heartbeat / Review / Settlement。它只处理状态转换。

## 生命周期

```text
register(identity)
  -> agents 插入或更新
  -> lifecycle = Registered
  -> 不自动进入 alive_agents
  -> 不更新 capability_index

declare_capabilities(agent_id, capabilities)
  -> 校验 Agent 已注册且未 deregister
  -> 移除旧 capability_index
  -> 写入新 capabilities
  -> 插入新 capability_index
  -> 不自动进入 alive_agents

AgentRecovered 或首次 heartbeat 接入事件
  -> mark_alive(agent_id)
  -> 加入 alive_agents

AgentTimedOut
  -> mark_timed_out(agent_id)
  -> 从 alive_agents 移除

deregister(agent_id)
  -> lifecycle = Deregistered
  -> 从 alive_agents 移除
  -> 从 capability_index 移除
```

注册成功不等于可发现。Agent 还必须声明能力，并通过 Heartbeat 证明自己活着，Registry 才能在 `discover()` 中返回它。

## Heartbeat 集成

Registry 通过事件消费 Heartbeat 状态，不反向调用 Heartbeat：

| Heartbeat 事件 | Registry 行为 |
|---------------|---------------|
| `AgentTimedOut { agent_id }` | `mark_timed_out(agent_id)`，让 Agent 不可发现 |
| `AgentRecovered { agent_id }` | `mark_alive(agent_id)`，若已注册且未 deregister，则恢复可发现 |

未知 Agent 的 Heartbeat 事件应该被忽略并记录日志。这样可以容忍事件乱序，例如 Agent 先 ping，稍后才 register。

## Discovery

```rust
pub struct DiscoveryQuery {
    pub capability: CapabilityName,
    pub include_busy: bool,
    pub limit: Option<usize>,
}

pub struct AgentCandidate {
    pub agent_id: AgentId,
    pub name: Option<String>,
    pub endpoint: Option<String>,
    pub capability: Capability,
    pub current_load: u32,
    pub max_concurrency: u32,
}
```

Discovery 第一版只返回 Registry 本地强一致数据。Review / Settlement 数据暂不放进 `RegistryCore`，后续用独立 enrichment 层补：

```text
Registry.discover(query)
  -> Vec<AgentCandidate>
  -> DiscoveryEnricher 补 review / settlement / metrics
  -> AgentSnapshot
```

这样 `discover()` 不会因为链上查询、历史统计或外部服务抖动而拖慢核心能力发现。

复杂度：

```text
O(1) capability index lookup
+ O(k) candidate filtering
+ O(k log k) optional deterministic ordering
```

`k` 是声明该能力的 Agent 数量，不是全市场 Agent 数量。

## 负载与容量

Registry 不执行任务，但可以维护负载视图：

```rust
set_load(agent_id, current)
```

调用来源可以是后续 Task / Assignment 组件。Registry 只校验：

- Agent 必须存在且未 deregister
- `current` 不能超过 Agent 所有能力的最大并发上限中的最大值

`discover()` 默认过滤满载 Agent：

```text
current_load < capability.max_concurrency
```

如果调用方设置 `include_busy = true`，可以返回满载 Agent，但候选项必须带上 `current_load`，由买方 Agent 自己决策。

## Artifact 协议契约扩展

当前 `Capability` 包含能力名、并发上限和可选 Artifact 协议契约：

```rust
pub struct Capability {
    pub name: CapabilityName,
    pub max_concurrency: u32,
    pub contract: Option<CapabilityContract>,
}

pub struct CapabilityContract {
    pub input_profiles: Vec<MediaProfileId>,
    pub output_profiles: Vec<MediaProfileId>,
}
```

示例：

```text
video.review
  input_profiles:  ["video.mp4.h264-aac.v1"]
  output_profiles: ["application.vnd.agent.review-verdict-json.v1"]
```

Registry 只保存和发现这些契约，不负责校验文件内容，也不负责转码。发起 Agent 根据 `output_profiles -> input_profiles` 做链路兼容匹配；不兼容时选择 transformer Agent。

能力声明失败不能部分更新索引。实现上先完整校验 `Vec<Capability>`，包括 contract 内 profile 是否受支持和是否重复，再移除旧索引并写入新索引。

## 错误处理

```rust
pub enum RegistryError {
    EmptyCapabilityList,
    DuplicateCapability(CapabilityName),
    ZeroMaxConcurrency(CapabilityName),
    DuplicateMediaProfile(MediaProfileId),
    UnsupportedMediaProfile(MediaProfileId),
    AgentNotFound(AgentId),
    AgentDeregistered(AgentId),
    LoadExceedsCapacity { agent_id: AgentId, current: u32, max: u32 },
}
```

注册失败不能部分更新状态。实现上先校验完整 `AgentIdentity`，再执行写入。

重复注册同一个 Agent 是合法操作，语义是替换基础信息，不影响已有能力声明、负载和 Heartbeat alive 状态。

能力声明失败不能部分更新索引。实现上先校验完整 `Vec<Capability>`，包括能力名、并发上限、contract profile 是否受支持和是否重复，再移除旧索引并写入新索引。

## 服务层

```rust
pub enum RegistryCommand {
    Register { identity: AgentIdentity, reply: oneshot::Sender<Result<RegisterOutcome, RegistryError>> },
    DeclareCapabilities { agent_id: AgentId, capabilities: Vec<Capability>, reply: oneshot::Sender<Result<CapabilityUpdateOutcome, RegistryError>> },
    Deregister { agent_id: AgentId, reply: oneshot::Sender<bool> },
    MarkAlive { agent_id: AgentId },
    MarkTimedOut { agent_id: AgentId },
    SetLoad { agent_id: AgentId, current: u32, reply: oneshot::Sender<Result<(), RegistryError>> },
    Discover { query: DiscoveryQuery, reply: oneshot::Sender<Vec<AgentCandidate>> },
    Shutdown { reply: oneshot::Sender<()> },
}
```

服务层职责：

- 顺序处理命令，避免锁扩散
- 消费 Heartbeat 事件并转换为 `MarkAlive` / `MarkTimedOut`
- 对未知事件只记录，不 panic
- `Shutdown` 时退出循环后再确认调用方

## 测试策略

核心测试优先：

- 注册 Agent 后 `get()` 可读
- 注册 Agent 后未声明能力不可发现
- 注册 Agent 且声明能力后，未 heartbeat 仍不可发现
- `mark_alive()` 后可发现
- `AgentTimedOut` 后不可发现但 AgentInfo 仍存在
- `AgentRecovered` 后恢复可发现
- `deregister()` 后不可发现，后续 `mark_alive()` 不恢复
- 重复注册只替换基础信息，不清空能力索引
- 能力声明会替换旧能力集合，并清理旧能力索引
- 空能力列表被拒绝
- 重复能力被拒绝
- `max_concurrency = 0` 被拒绝
- capability contract 会随 discovery 返回
- contract 内未知 media profile 被拒绝，且不污染旧索引
- contract 内重复 media profile 被拒绝
- 满载 Agent 默认不出现在 `discover()`
- `include_busy = true` 时可以返回满载 Agent
- 多 Agent 同能力发现只扫描该能力索引

服务测试后置：

- register / discover 命令往返正常
- Heartbeat `AgentTimedOut` 事件能更新 Registry
- Heartbeat `AgentRecovered` 事件能恢复 Registry
- 未知 Heartbeat 事件不 panic
- shutdown 后 handle 返回 stopped

## 暂不做

- Review / Settlement 实时聚合
- Agent 评分和排名
- 分布式 Registry
- 持久化注册表
- 权限和签名校验
- 网络 API

这些属于后续组件或运行时层。第一版先把单进程内注册、索引、发现和 Heartbeat 事件联动做稳定。
