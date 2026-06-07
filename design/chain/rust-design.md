# Chain Rust Design

## 目标

Chain 是任务链路账本。它让平台能追踪任务从哪个 Agent 流向哪个 Agent，以及每个节点提交了哪个可验证 artifact。平台不保存 artifact 内容，只保存 hash、引用、holder 承诺和签名。

## 设计选择

采用和 Heartbeat / Registry 一致的结构：

```text
ChainCore      纯状态机
ChainService   Tokio 命令循环
```

`ChainCore` 不读系统时间，不访问网络，不拉取 artifact。它只维护链路、manifest、commitment 和节点状态。

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
    pub agent_id: AgentId,
    pub previous: Option<NodeId>,
    pub next: Option<NodeId>,
    pub input: Option<ArtifactRef>,
    pub output: Option<ArtifactRef>,
    pub status: NodeStatus,
}
```

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
    pub fn create_chain(&mut self, task_id: TaskId, root_agent: AgentId) -> ChainId;
    pub fn append_node(
        &mut self,
        chain_id: &ChainId,
        previous: NodeId,
        agent_id: AgentId,
        input: ArtifactRef,
    ) -> Result<NodeId, ChainError>;
    pub fn register_artifact(&mut self, manifest: ArtifactManifest) -> Result<(), ChainError>;
    pub fn add_holder(&mut self, commitment: HolderCommitment) -> Result<(), ChainError>;
    pub fn submit_output(&mut self, node_id: &NodeId, output: ArtifactRef) -> Result<(), ChainError>;
    pub fn close_chain(&mut self, chain_id: &ChainId, final_node: &NodeId) -> Result<(), ChainError>;
    pub fn get_chain(&self, chain_id: &ChainId) -> Option<ChainSnapshot>;
}
```

## 不变量

- 一个 `ChainNode` 只能属于一个 `TaskChain`
- `append_node` 必须连接到当前 head，避免分叉
- `append_node` 要求 previous node 已有 output
- `append_node.input` 必须等于 previous node 的 output
- `submit_output` 的 artifact 必须已注册 manifest
- `add_holder` 的 artifact 必须已注册 manifest
- `close_chain` 只能在 final node 有 output 后执行
- 已关闭链不能继续追加节点
- 平台只校验 hash / 签名 / 引用结构，不校验正文内容

这些不变量保证链路结构和内容引用一致。否则平台虽然能看到 A -> B -> C 的结构，但无法证明 B 的输入确实来自 A 的输出。

## 跨组件联动

`ChainCore` 和 `ChainService` 第一版不直接依赖 Registry、Heartbeat、Review、Settlement。它们只暴露链路账本能力，跨组件联动由上层 runtime / scheduler 负责。

典型流程：

```text
Registry.discover(capability)
  -> 调用方选择 Agent B
  -> Agent A 将 artifact_x 交给 B，或 B 从 holder 拉取 artifact_x
  -> Chain.register_artifact(artifact_x)
  -> Chain.add_holder(holder commitment)
  -> Chain.submit_output(A_node, artifact_x)
  -> Chain.append_node(previous=A_node, agent=B, input=artifact_x)
```

掉线流程：

```text
Heartbeat.scan()
  -> AgentTimedOut(agent_id)
  -> Registry.mark_timed_out(agent_id)
  -> scheduler 查询相关 chain node
  -> Settlement refund / scheduler retry / Review 标记 artifact 拉取失败
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
    ChainNotFound(ChainId),
    NodeNotFound(NodeId),
    ArtifactNotFound(ArtifactId),
    ArtifactHashMismatch { artifact_id: ArtifactId, expected: Hash, actual: Hash },
    ChainClosed(ChainId),
    NotChainHead { expected: NodeId, actual: NodeId },
    PreviousNodeMissingOutput(NodeId),
    InputDoesNotMatchPreviousOutput { previous: NodeId, expected: ArtifactRef, actual: ArtifactRef },
    NodeAlreadyHasOutput(NodeId),
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

第一版只做可验证链路和 artifact holder 账本。
