# Chain Rust Design

## 目标

Chain 是任务链路账本。它让平台能追踪任务从哪个 Agent 流向哪个 Agent，每个节点由谁执行、谁审查，以及每个节点提交了哪个可验证 artifact。

平台不保存 artifact 内容，只保存 hash、引用、holder 承诺和签名。

## 设计选择

采用和 Heartbeat / Registry 一致的结构：

```text
ChainCore      纯状态机
ChainService   Tokio 命令循环
```

`ChainCore` 不读系统时间，不访问网络，不拉取 artifact，不查询 Registry / Review / Settlement。它只维护链路、节点角色、manifest、commitment 和节点状态。

跨组件联动由上层 runtime / scheduler 编排。

## 模块结构

```text
src/
├── chain/
│   ├── mod.rs
│   ├── core.rs
│   ├── service.rs
│   └── types.rs
├── registry/
├── heartbeat/
├── review/
└── settlement/
```

## 数据模型

```rust
pub struct ChainCore {
    chains: HashMap<ChainId, TaskChain>,
    nodes: HashMap<NodeId, ChainNode>,
    artifacts: HashMap<ArtifactId, ArtifactManifest>,
    holders: HashMap<ArtifactId, Vec<HolderCommitment>>,
}

pub struct TaskChain {
    pub chain_id: ChainId,
    pub task_id: TaskId,
    pub root_agent: AgentId,
    pub head: NodeId,
    pub status: ChainStatus,
}

pub struct ChainNode {
    pub node_id: NodeId,
    pub chain_id: ChainId,
    pub executor: AgentId,
    pub reviewers: Vec<AgentId>,
    pub previous: Option<NodeId>,
    pub next: Option<NodeId>,
    pub input: Option<ArtifactRef>,
    pub output: Option<ArtifactRef>,
    pub status: NodeStatus,
}
```

`executor` 是产出 artifact 的 Agent。`reviewers` 是允许审阅该节点 output 的 Agent 快照来源。

## Artifact

```rust
pub struct ArtifactManifest {
    pub artifact_id: ArtifactId,
    pub root_hash: Hash,
    pub size_bytes: u64,
    pub content_type: String,
    pub created_by: AgentId,
}

pub struct ArtifactRef {
    pub artifact_id: ArtifactId,
    pub root_hash: Hash,
}

pub struct HolderCommitment {
    pub artifact_id: ArtifactId,
    pub holder_agent: AgentId,
    pub retrieval_endpoint: String,
    pub expires_at: Timestamp,
    pub signature: Signature,
}
```

第一版不做 chunk 分布式存储。`root_hash` 是完整 artifact 的内容 hash。后续可以扩展 `ChunkManifest`，但不影响节点输出引用。

## 核心原语

```rust
impl ChainCore {
    pub fn create_chain(
        &mut self,
        task_id: TaskId,
        root_agent: AgentId,
        reviewers: Vec<AgentId>,
    ) -> Result<ChainId, ChainError>;

    pub fn append_node(
        &mut self,
        chain_id: &ChainId,
        previous: NodeId,
        executor: AgentId,
        reviewers: Vec<AgentId>,
        input: ArtifactRef,
    ) -> Result<NodeId, ChainError>;

    pub fn assign_executor(
        &mut self,
        node_id: &NodeId,
        executor: AgentId,
    ) -> Result<(), ChainError>;

    pub fn assign_reviewers(
        &mut self,
        node_id: &NodeId,
        reviewers: Vec<AgentId>,
    ) -> Result<(), ChainError>;

    pub fn register_artifact(&mut self, manifest: ArtifactManifest) -> Result<(), ChainError>;
    pub fn add_holder(&mut self, commitment: HolderCommitment) -> Result<(), ChainError>;
    pub fn submit_output(&mut self, node_id: &NodeId, output: ArtifactRef) -> Result<(), ChainError>;
    pub fn close_chain(&mut self, chain_id: &ChainId) -> Result<(), ChainError>;
    pub fn get_chain(&self, chain_id: &ChainId) -> Option<ChainSnapshot>;
}
```

`create_chain` 返回 `Result`，因为 reviewers 需要校验去重，executor/root_agent 也需要校验基础合法性。

## 不变量

- 一个 `ChainNode` 只能属于一个 `TaskChain`
- `executor` 必须存在且不能为空
- `reviewers` 内不能重复
- `append_node` 必须连接到当前 head，避免分叉
- `append_node` 要求 previous node 已有 output
- `append_node.input` 必须等于 previous node 的 output
- `submit_output` 的 artifact 必须已注册 manifest
- `add_holder` 的 artifact 必须已注册 manifest
- 同一个节点只能提交一次 output
- 节点已有 output 后不能再 `assign_executor`
- `assign_reviewers` 不修改已有 ReviewSession
- `close_chain` 只检查所有节点都有 output，不检查 Review
- 已关闭链不能继续追加节点或修改节点角色
- 平台只校验 hash / 签名 / 引用结构，不校验正文内容

## 跨组件联动

`ChainCore` 和 `ChainService` 第一版不直接依赖 Registry、Heartbeat、Review、Settlement。它们只暴露链路账本能力，跨组件联动由上层 runtime / scheduler 负责。

典型流程：

```text
Registry.discover("code-analysis")
  -> 调用方选择 executor B

Registry.discover("review:code-analysis")
  -> 调用方选择 reviewers [R1, R2]

Agent A 将 artifact_x 交给 B，或 B 从 holder 拉取 artifact_x
  -> Chain.register_artifact(artifact_x)
  -> Chain.add_holder(holder commitment)
  -> Chain.submit_output(A_node, artifact_x)
  -> Chain.append_node(previous=A_node, executor=B, reviewers=[R1, R2], input=artifact_x)
```

审阅流程：

```text
Chain.get_chain(chain_id)
  -> node.output
  -> node.reviewers
  -> Review.request(node_id, artifact_ref, reviewers_snapshot, criteria)
```

掉线流程：

```text
Heartbeat.scan()
  -> AgentTimedOut(agent_id)
  -> Registry.mark_timed_out(agent_id)
  -> scheduler 查询相关 chain node
  -> 未产出节点: Chain.assign_executor(...)
  -> 未创建 review session: Chain.assign_reviewers(...)
  -> 已创建 review session: Review 新建替代 session
  -> Settlement refund / scheduler retry
```

这里的关键边界是：Chain 不判断 Agent 是否 alive，也不做重新派单；它只提供可查询的链路和 artifact 事实。

## 读取内容

平台查询链路：

```text
get_chain(chain_id)
  -> ChainSnapshot
  -> node.output.artifact_id
  -> holders[artifact_id]
```

调用方再向 holder 拉取内容：

```text
GET holder.retrieval_endpoint/artifacts/{artifact_id}
```

拉回后必须校验：

```text
sha256(content) == ArtifactManifest.root_hash
```

校验失败时，内容不可信；Chain 本身仍然可证明当时提交的 hash 是什么。

## 错误处理

```rust
pub enum ChainError {
    EmptyId(&'static str),
    DuplicateReviewer(AgentId),
    ChainNotFound(ChainId),
    NodeNotFound(NodeId),
    ArtifactNotFound(ArtifactId),
    ArtifactHashMismatch { artifact_id: ArtifactId, expected: Hash, actual: Hash },
    ChainClosed(ChainId),
    NotChainHead { expected: NodeId, actual: NodeId },
    PreviousNodeMissingOutput(NodeId),
    InputDoesNotMatchPreviousOutput { previous: NodeId, expected: ArtifactRef, actual: ArtifactRef },
    NodeAlreadyHasOutput(NodeId),
    NodeAlreadyCompleted(NodeId),
    FinalNodeMissingOutput(NodeId),
    DuplicateArtifact(ArtifactId),
    DuplicateHolder { artifact_id: ArtifactId, holder_agent: AgentId },
    EmptyContentType,
    EmptyHolderEndpoint,
}
```

所有写操作必须先校验再更新，避免部分写入。

## 暂不做

- 平台托管 artifact
- chunk mesh
- DHT / peer discovery
- 自动补副本
- 存储证明 challenge
- 存储奖励 / 惩罚
- Chain 内部直接检查 Review 是否完成

第一版只做可验证链路、节点角色和 artifact holder 账本。
