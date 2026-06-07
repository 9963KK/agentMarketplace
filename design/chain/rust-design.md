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
- `submit_output` 的 artifact 必须已注册 manifest
- `add_holder` 的 artifact 必须已注册 manifest
- `close_chain` 只能在 final node 有 output 后执行
- 已关闭链不能继续追加节点
- 平台只校验 hash / 签名 / 引用结构，不校验正文内容

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
    ChainClosed(ChainId),
    NotChainHead { expected: NodeId, actual: NodeId },
    NodeAlreadyHasOutput(NodeId),
    FinalNodeMissingOutput(NodeId),
    DuplicateArtifact(ArtifactId),
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
