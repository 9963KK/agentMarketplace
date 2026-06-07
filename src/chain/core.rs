use std::collections::{HashMap, HashSet};

use crate::heartbeat::AgentId;

use super::types::{
    ArtifactId, ArtifactManifest, ArtifactRef, ChainError, ChainId, ChainNode, ChainSnapshot,
    ChainStatus, HolderCommitment, NodeId, NodeStatus, TaskChain, TaskId,
};

#[derive(Debug, Default)]
pub struct ChainCore {
    chains: HashMap<ChainId, TaskChain>,
    nodes: HashMap<NodeId, ChainNode>,
    artifacts: HashMap<ArtifactId, ArtifactManifest>,
    holders: HashMap<ArtifactId, Vec<HolderCommitment>>,
    next_chain: u64,
    next_node: u64,
}

impl ChainCore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_chain(
        &mut self,
        task_id: TaskId,
        root_agent: AgentId,
        reviewers: Vec<AgentId>,
    ) -> Result<ChainId, ChainError> {
        validate_reviewers(&reviewers)?;

        let chain_id = self.next_chain_id();
        let root_node_id = self.next_node_id();

        self.nodes.insert(
            root_node_id.clone(),
            ChainNode {
                node_id: root_node_id.clone(),
                chain_id: chain_id.clone(),
                executor: root_agent.clone(),
                reviewers,
                previous: None,
                next: None,
                input: None,
                output: None,
                status: NodeStatus::Pending,
            },
        );
        self.chains.insert(
            chain_id.clone(),
            TaskChain {
                chain_id: chain_id.clone(),
                task_id,
                root_agent,
                head: root_node_id,
                status: ChainStatus::Open,
            },
        );

        Ok(chain_id)
    }

    pub fn append_node(
        &mut self,
        chain_id: &ChainId,
        previous: NodeId,
        executor: AgentId,
        reviewers: Vec<AgentId>,
        input: ArtifactRef,
    ) -> Result<NodeId, ChainError> {
        validate_reviewers(&reviewers)?;
        self.ensure_artifact_ref_known(&input)?;

        let chain = self
            .chains
            .get(chain_id)
            .ok_or_else(|| ChainError::ChainNotFound(chain_id.clone()))?;
        if chain.status == ChainStatus::Closed {
            return Err(ChainError::ChainClosed(chain_id.clone()));
        }
        if chain.head != previous {
            return Err(ChainError::NotChainHead {
                expected: chain.head.clone(),
                actual: previous,
            });
        }

        let previous_output = self
            .nodes
            .get(&previous)
            .ok_or_else(|| ChainError::NodeNotFound(previous.clone()))?
            .output
            .clone()
            .ok_or_else(|| ChainError::PreviousNodeMissingOutput(previous.clone()))?;
        if previous_output != input {
            return Err(ChainError::InputDoesNotMatchPreviousOutput {
                previous,
                expected: previous_output,
                actual: input,
            });
        }

        let node_id = self.next_node_id();
        let previous_node = self
            .nodes
            .get_mut(&previous)
            .ok_or_else(|| ChainError::NodeNotFound(previous.clone()))?;
        previous_node.next = Some(node_id.clone());

        self.nodes.insert(
            node_id.clone(),
            ChainNode {
                node_id: node_id.clone(),
                chain_id: chain_id.clone(),
                executor,
                reviewers,
                previous: Some(previous),
                next: None,
                input: Some(input),
                output: None,
                status: NodeStatus::Pending,
            },
        );

        let chain = self
            .chains
            .get_mut(chain_id)
            .ok_or_else(|| ChainError::ChainNotFound(chain_id.clone()))?;
        chain.head = node_id.clone();

        Ok(node_id)
    }

    pub fn assign_executor(
        &mut self,
        node_id: &NodeId,
        executor: AgentId,
    ) -> Result<(), ChainError> {
        let chain_id = self
            .nodes
            .get(node_id)
            .ok_or_else(|| ChainError::NodeNotFound(node_id.clone()))?
            .chain_id
            .clone();
        self.ensure_chain_open(&chain_id)?;

        let node = self
            .nodes
            .get_mut(node_id)
            .ok_or_else(|| ChainError::NodeNotFound(node_id.clone()))?;
        if node.output.is_some() {
            return Err(ChainError::NodeAlreadyCompleted(node_id.clone()));
        }

        node.executor = executor;
        Ok(())
    }

    pub fn assign_reviewers(
        &mut self,
        node_id: &NodeId,
        reviewers: Vec<AgentId>,
    ) -> Result<(), ChainError> {
        validate_reviewers(&reviewers)?;

        let chain_id = self
            .nodes
            .get(node_id)
            .ok_or_else(|| ChainError::NodeNotFound(node_id.clone()))?
            .chain_id
            .clone();
        self.ensure_chain_open(&chain_id)?;

        let node = self
            .nodes
            .get_mut(node_id)
            .ok_or_else(|| ChainError::NodeNotFound(node_id.clone()))?;
        node.reviewers = reviewers;
        Ok(())
    }

    pub fn register_artifact(&mut self, manifest: ArtifactManifest) -> Result<(), ChainError> {
        validate_manifest(&manifest)?;
        if self.artifacts.contains_key(&manifest.artifact_id) {
            return Err(ChainError::DuplicateArtifact(manifest.artifact_id));
        }

        self.artifacts
            .insert(manifest.artifact_id.clone(), manifest);
        Ok(())
    }

    pub fn add_holder(&mut self, commitment: HolderCommitment) -> Result<(), ChainError> {
        if commitment.retrieval_endpoint.trim().is_empty() {
            return Err(ChainError::EmptyHolderEndpoint);
        }
        self.artifacts
            .get(&commitment.artifact_id)
            .ok_or_else(|| ChainError::ArtifactNotFound(commitment.artifact_id.clone()))?;

        let holders = self
            .holders
            .entry(commitment.artifact_id.clone())
            .or_default();
        if holders
            .iter()
            .any(|holder| holder.holder_agent == commitment.holder_agent)
        {
            return Err(ChainError::DuplicateHolder {
                artifact_id: commitment.artifact_id,
                holder_agent: commitment.holder_agent,
            });
        }

        holders.push(commitment);
        Ok(())
    }

    pub fn submit_output(
        &mut self,
        node_id: &NodeId,
        output: ArtifactRef,
    ) -> Result<(), ChainError> {
        self.ensure_artifact_ref_known(&output)?;

        let node = self
            .nodes
            .get_mut(node_id)
            .ok_or_else(|| ChainError::NodeNotFound(node_id.clone()))?;
        if node.output.is_some() {
            return Err(ChainError::NodeAlreadyHasOutput(node_id.clone()));
        }

        node.output = Some(output);
        node.status = NodeStatus::Completed;
        Ok(())
    }

    pub fn close_chain(&mut self, chain_id: &ChainId) -> Result<(), ChainError> {
        let chain = self
            .chains
            .get(chain_id)
            .ok_or_else(|| ChainError::ChainNotFound(chain_id.clone()))?;
        if chain.status == ChainStatus::Closed {
            return Err(ChainError::ChainClosed(chain_id.clone()));
        }

        for node in self.nodes_for_chain(chain) {
            if node.output.is_none() {
                return Err(ChainError::FinalNodeMissingOutput(node.node_id));
            }
        }

        let chain = self
            .chains
            .get_mut(chain_id)
            .ok_or_else(|| ChainError::ChainNotFound(chain_id.clone()))?;
        chain.status = ChainStatus::Closed;
        Ok(())
    }

    pub fn get_chain(&self, chain_id: &ChainId) -> Option<ChainSnapshot> {
        let chain = self.chains.get(chain_id)?.clone();
        let nodes = self.nodes_for_chain(&chain);
        let artifacts = artifacts_for_nodes(&nodes)
            .into_iter()
            .filter_map(|artifact_id| self.artifacts.get(&artifact_id).cloned())
            .collect::<Vec<_>>();
        let holders = artifacts
            .iter()
            .flat_map(|artifact| {
                self.holders
                    .get(&artifact.artifact_id)
                    .cloned()
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();

        Some(ChainSnapshot {
            chain,
            nodes,
            artifacts,
            holders,
        })
    }

    pub fn chain_count(&self) -> usize {
        self.chains.len()
    }

    pub fn artifact_count(&self) -> usize {
        self.artifacts.len()
    }

    fn ensure_artifact_ref_known(&self, artifact_ref: &ArtifactRef) -> Result<(), ChainError> {
        let manifest = self
            .artifacts
            .get(&artifact_ref.artifact_id)
            .ok_or_else(|| ChainError::ArtifactNotFound(artifact_ref.artifact_id.clone()))?;
        if manifest.root_hash != artifact_ref.root_hash {
            return Err(ChainError::ArtifactHashMismatch {
                artifact_id: artifact_ref.artifact_id.clone(),
                expected: manifest.root_hash.clone(),
                actual: artifact_ref.root_hash.clone(),
            });
        }

        Ok(())
    }

    fn ensure_chain_open(&self, chain_id: &ChainId) -> Result<(), ChainError> {
        let chain = self
            .chains
            .get(chain_id)
            .ok_or_else(|| ChainError::ChainNotFound(chain_id.clone()))?;
        if chain.status == ChainStatus::Closed {
            return Err(ChainError::ChainClosed(chain_id.clone()));
        }

        Ok(())
    }

    fn nodes_for_chain(&self, chain: &TaskChain) -> Vec<ChainNode> {
        let root = self
            .nodes
            .values()
            .find(|node| node.chain_id == chain.chain_id && node.previous.is_none());
        let Some(root) = root else {
            return Vec::new();
        };

        let mut nodes = Vec::new();
        let mut current = Some(root.node_id.clone());
        while let Some(node_id) = current {
            let Some(node) = self.nodes.get(&node_id) else {
                break;
            };
            nodes.push(node.clone());
            current = node.next.clone();
        }

        nodes
    }

    fn next_chain_id(&mut self) -> ChainId {
        self.next_chain += 1;
        ChainId::new(format!("chain-{}", self.next_chain))
    }

    fn next_node_id(&mut self) -> NodeId {
        self.next_node += 1;
        NodeId::new(format!("node-{}", self.next_node))
    }
}

fn validate_manifest(manifest: &ArtifactManifest) -> Result<(), ChainError> {
    if manifest.size_bytes == 0 {
        return Err(ChainError::ZeroArtifactSize(manifest.artifact_id.clone()));
    }
    if manifest.content_type.trim().is_empty() {
        return Err(ChainError::EmptyContentType);
    }

    Ok(())
}

fn validate_reviewers(reviewers: &[AgentId]) -> Result<(), ChainError> {
    let mut seen = HashSet::new();
    for reviewer in reviewers {
        if !seen.insert(reviewer.clone()) {
            return Err(ChainError::DuplicateReviewer(reviewer.clone()));
        }
    }

    Ok(())
}

fn artifacts_for_nodes(nodes: &[ChainNode]) -> Vec<ArtifactId> {
    let mut seen = HashSet::new();
    let mut artifacts = Vec::new();

    for artifact_ref in nodes
        .iter()
        .flat_map(|node| [node.input.as_ref(), node.output.as_ref()])
        .flatten()
    {
        if seen.insert(artifact_ref.artifact_id.clone()) {
            artifacts.push(artifact_ref.artifact_id.clone());
        }
    }

    artifacts
}

#[cfg(test)]
mod tests {
    use crate::chain::{Hash, Signature, Timestamp};

    use super::*;

    fn agent(id: &str) -> AgentId {
        AgentId::from(id)
    }

    fn task(id: &str) -> TaskId {
        TaskId::from(id)
    }

    fn artifact_id(id: &str) -> ArtifactId {
        ArtifactId::from(id)
    }

    fn hash(value: &str) -> Hash {
        Hash::from(value)
    }

    fn artifact_ref(id: &str, root_hash: &str) -> ArtifactRef {
        ArtifactRef {
            artifact_id: artifact_id(id),
            root_hash: hash(root_hash),
        }
    }

    fn manifest(id: &str, root_hash: &str, creator: &str) -> ArtifactManifest {
        ArtifactManifest {
            artifact_id: artifact_id(id),
            root_hash: hash(root_hash),
            size_bytes: 128,
            content_type: "application/json".to_string(),
            created_by: agent(creator),
        }
    }

    fn holder(id: &str, holder: &str) -> HolderCommitment {
        HolderCommitment {
            artifact_id: artifact_id(id),
            holder_agent: agent(holder),
            retrieval_endpoint: format!("https://{holder}.example.test/artifacts/{id}"),
            expires_at: Timestamp(10),
            signature: Signature::from(format!("sig-{holder}-{id}")),
        }
    }

    fn create_chain_with_root(core: &mut ChainCore) -> (ChainId, NodeId) {
        let chain_id = core
            .create_chain(task("task-1"), agent("agent-a"), vec![agent("reviewer-1")])
            .unwrap();
        let root = core.get_chain(&chain_id).unwrap().chain.head;

        (chain_id, root)
    }

    #[test]
    fn create_chain_creates_root_node_snapshot() {
        let mut core = ChainCore::new();

        let chain_id = core
            .create_chain(task("task-1"), agent("agent-a"), vec![agent("reviewer-1")])
            .unwrap();
        let snapshot = core.get_chain(&chain_id).unwrap();

        assert_eq!(core.chain_count(), 1);
        assert_eq!(snapshot.chain.task_id, task("task-1"));
        assert_eq!(snapshot.chain.root_agent, agent("agent-a"));
        assert_eq!(snapshot.chain.status, ChainStatus::Open);
        assert_eq!(snapshot.nodes.len(), 1);
        assert_eq!(snapshot.nodes[0].executor, agent("agent-a"));
        assert_eq!(snapshot.nodes[0].reviewers, vec![agent("reviewer-1")]);
        assert_eq!(snapshot.nodes[0].previous, None);
        assert_eq!(snapshot.nodes[0].status, NodeStatus::Pending);
    }

    #[test]
    fn create_chain_rejects_duplicate_reviewers() {
        let mut core = ChainCore::new();

        assert_eq!(
            core.create_chain(
                task("task-1"),
                agent("agent-a"),
                vec![agent("reviewer-1"), agent("reviewer-1")],
            )
            .unwrap_err(),
            ChainError::DuplicateReviewer(agent("reviewer-1"))
        );
    }

    #[test]
    fn register_artifact_rejects_duplicate_and_invalid_manifest() {
        let mut core = ChainCore::new();

        core.register_artifact(manifest("artifact-a", "hash-a", "agent-a"))
            .unwrap();

        assert_eq!(
            core.register_artifact(manifest("artifact-a", "hash-a", "agent-a"))
                .unwrap_err(),
            ChainError::DuplicateArtifact(artifact_id("artifact-a"))
        );

        let mut empty_content_type = manifest("artifact-b", "hash-b", "agent-a");
        empty_content_type.content_type = " ".to_string();
        assert_eq!(
            core.register_artifact(empty_content_type).unwrap_err(),
            ChainError::EmptyContentType
        );

        let mut zero_size = manifest("artifact-c", "hash-c", "agent-a");
        zero_size.size_bytes = 0;
        assert_eq!(
            core.register_artifact(zero_size).unwrap_err(),
            ChainError::ZeroArtifactSize(artifact_id("artifact-c"))
        );
    }

    #[test]
    fn add_holder_requires_registered_artifact_and_unique_holder() {
        let mut core = ChainCore::new();

        assert_eq!(
            core.add_holder(holder("artifact-a", "agent-a"))
                .unwrap_err(),
            ChainError::ArtifactNotFound(artifact_id("artifact-a"))
        );

        core.register_artifact(manifest("artifact-a", "hash-a", "agent-a"))
            .unwrap();
        core.add_holder(holder("artifact-a", "agent-a")).unwrap();

        assert_eq!(
            core.add_holder(holder("artifact-a", "agent-a"))
                .unwrap_err(),
            ChainError::DuplicateHolder {
                artifact_id: artifact_id("artifact-a"),
                holder_agent: agent("agent-a"),
            }
        );

        let mut empty_endpoint = holder("artifact-a", "agent-b");
        empty_endpoint.retrieval_endpoint = " ".to_string();
        assert_eq!(
            core.add_holder(empty_endpoint).unwrap_err(),
            ChainError::EmptyHolderEndpoint
        );
    }

    #[test]
    fn append_node_requires_registered_input_artifact() {
        let mut core = ChainCore::new();
        let (chain_id, root) = create_chain_with_root(&mut core);

        assert_eq!(
            core.append_node(
                &chain_id,
                root,
                agent("agent-b"),
                vec![agent("reviewer-b")],
                artifact_ref("artifact-a", "hash-a"),
            )
            .unwrap_err(),
            ChainError::ArtifactNotFound(artifact_id("artifact-a"))
        );
    }

    #[test]
    fn append_node_requires_previous_output() {
        let mut core = ChainCore::new();
        let (chain_id, root) = create_chain_with_root(&mut core);
        core.register_artifact(manifest("artifact-a", "hash-a", "agent-a"))
            .unwrap();

        assert_eq!(
            core.append_node(
                &chain_id,
                root.clone(),
                agent("agent-b"),
                vec![agent("reviewer-b")],
                artifact_ref("artifact-a", "hash-a"),
            )
            .unwrap_err(),
            ChainError::PreviousNodeMissingOutput(root)
        );
    }

    #[test]
    fn append_node_requires_input_to_match_previous_output() {
        let mut core = ChainCore::new();
        let (chain_id, root) = create_chain_with_root(&mut core);
        core.register_artifact(manifest("artifact-a", "hash-a", "agent-a"))
            .unwrap();
        core.register_artifact(manifest("artifact-b", "hash-b", "agent-a"))
            .unwrap();
        core.submit_output(&root, artifact_ref("artifact-a", "hash-a"))
            .unwrap();

        assert_eq!(
            core.append_node(
                &chain_id,
                root.clone(),
                agent("agent-b"),
                vec![agent("reviewer-b")],
                artifact_ref("artifact-b", "hash-b"),
            )
            .unwrap_err(),
            ChainError::InputDoesNotMatchPreviousOutput {
                previous: root,
                expected: artifact_ref("artifact-a", "hash-a"),
                actual: artifact_ref("artifact-b", "hash-b"),
            }
        );
    }

    #[test]
    fn append_node_must_extend_current_head() {
        let mut core = ChainCore::new();
        let (chain_id, root) = create_chain_with_root(&mut core);
        core.register_artifact(manifest("artifact-a", "hash-a", "agent-a"))
            .unwrap();
        core.register_artifact(manifest("artifact-b", "hash-b", "agent-b"))
            .unwrap();
        core.submit_output(&root, artifact_ref("artifact-a", "hash-a"))
            .unwrap();
        let node_b = core
            .append_node(
                &chain_id,
                root.clone(),
                agent("agent-b"),
                vec![agent("reviewer-b")],
                artifact_ref("artifact-a", "hash-a"),
            )
            .unwrap();

        assert_eq!(
            core.append_node(
                &chain_id,
                root.clone(),
                agent("agent-c"),
                vec![agent("reviewer-c")],
                artifact_ref("artifact-a", "hash-a"),
            )
            .unwrap_err(),
            ChainError::NotChainHead {
                expected: node_b,
                actual: root,
            }
        );
    }

    #[test]
    fn submit_output_requires_registered_artifact_and_matching_hash() {
        let mut core = ChainCore::new();
        let (_, root) = create_chain_with_root(&mut core);

        assert_eq!(
            core.submit_output(&root, artifact_ref("artifact-a", "hash-a"))
                .unwrap_err(),
            ChainError::ArtifactNotFound(artifact_id("artifact-a"))
        );

        core.register_artifact(manifest("artifact-a", "hash-a", "agent-a"))
            .unwrap();
        assert_eq!(
            core.submit_output(&root, artifact_ref("artifact-a", "wrong"))
                .unwrap_err(),
            ChainError::ArtifactHashMismatch {
                artifact_id: artifact_id("artifact-a"),
                expected: hash("hash-a"),
                actual: hash("wrong"),
            }
        );
    }

    #[test]
    fn submit_output_rejects_duplicate_output() {
        let mut core = ChainCore::new();
        let (_, root) = create_chain_with_root(&mut core);
        core.register_artifact(manifest("artifact-a", "hash-a", "agent-a"))
            .unwrap();
        core.submit_output(&root, artifact_ref("artifact-a", "hash-a"))
            .unwrap();

        assert_eq!(
            core.submit_output(&root, artifact_ref("artifact-a", "hash-a"))
                .unwrap_err(),
            ChainError::NodeAlreadyHasOutput(root)
        );
    }

    #[test]
    fn assign_executor_and_reviewers_update_pending_node() {
        let mut core = ChainCore::new();
        let (chain_id, root) = create_chain_with_root(&mut core);
        core.register_artifact(manifest("artifact-a", "hash-a", "agent-a"))
            .unwrap();
        core.submit_output(&root, artifact_ref("artifact-a", "hash-a"))
            .unwrap();
        let node_b = core
            .append_node(
                &chain_id,
                root,
                agent("agent-b"),
                vec![agent("reviewer-b")],
                artifact_ref("artifact-a", "hash-a"),
            )
            .unwrap();

        core.assign_executor(&node_b, agent("agent-c")).unwrap();
        core.assign_reviewers(&node_b, vec![agent("reviewer-c")])
            .unwrap();
        let node = core
            .get_chain(&chain_id)
            .unwrap()
            .nodes
            .into_iter()
            .find(|node| node.node_id == node_b)
            .unwrap();

        assert_eq!(node.executor, agent("agent-c"));
        assert_eq!(node.reviewers, vec![agent("reviewer-c")]);
    }

    #[test]
    fn assign_executor_rejects_completed_node() {
        let mut core = ChainCore::new();
        let (_, root) = create_chain_with_root(&mut core);
        core.register_artifact(manifest("artifact-a", "hash-a", "agent-a"))
            .unwrap();
        core.submit_output(&root, artifact_ref("artifact-a", "hash-a"))
            .unwrap();

        assert_eq!(
            core.assign_executor(&root, agent("agent-b")).unwrap_err(),
            ChainError::NodeAlreadyCompleted(root)
        );
    }

    #[test]
    fn assign_reviewers_rejects_duplicates() {
        let mut core = ChainCore::new();
        let (_, root) = create_chain_with_root(&mut core);

        assert_eq!(
            core.assign_reviewers(&root, vec![agent("reviewer-1"), agent("reviewer-1")])
                .unwrap_err(),
            ChainError::DuplicateReviewer(agent("reviewer-1"))
        );
    }

    #[test]
    fn close_chain_requires_all_nodes_to_have_output() {
        let mut core = ChainCore::new();
        let (chain_id, root) = create_chain_with_root(&mut core);

        assert_eq!(
            core.close_chain(&chain_id).unwrap_err(),
            ChainError::FinalNodeMissingOutput(root.clone())
        );

        core.register_artifact(manifest("artifact-a", "hash-a", "agent-a"))
            .unwrap();
        core.submit_output(&root, artifact_ref("artifact-a", "hash-a"))
            .unwrap();
        let node_b = core
            .append_node(
                &chain_id,
                root.clone(),
                agent("agent-b"),
                vec![agent("reviewer-b")],
                artifact_ref("artifact-a", "hash-a"),
            )
            .unwrap();

        assert_eq!(
            core.close_chain(&chain_id).unwrap_err(),
            ChainError::FinalNodeMissingOutput(node_b)
        );
    }

    #[test]
    fn closed_chain_cannot_append() {
        let mut core = ChainCore::new();
        let (chain_id, root) = create_chain_with_root(&mut core);
        core.register_artifact(manifest("artifact-a", "hash-a", "agent-a"))
            .unwrap();
        core.submit_output(&root, artifact_ref("artifact-a", "hash-a"))
            .unwrap();
        core.close_chain(&chain_id).unwrap();

        assert_eq!(
            core.append_node(
                &chain_id,
                root,
                agent("agent-b"),
                vec![agent("reviewer-b")],
                artifact_ref("artifact-a", "hash-a"),
            )
            .unwrap_err(),
            ChainError::ChainClosed(chain_id)
        );
    }

    #[test]
    fn snapshot_returns_nodes_artifacts_holders_in_chain_order() {
        let mut core = ChainCore::new();
        let (chain_id, root) = create_chain_with_root(&mut core);
        core.register_artifact(manifest("artifact-a", "hash-a", "agent-a"))
            .unwrap();
        core.register_artifact(manifest("artifact-b", "hash-b", "agent-b"))
            .unwrap();
        core.add_holder(holder("artifact-a", "agent-a")).unwrap();
        core.add_holder(holder("artifact-b", "agent-b")).unwrap();

        core.submit_output(&root, artifact_ref("artifact-a", "hash-a"))
            .unwrap();
        let node_b = core
            .append_node(
                &chain_id,
                root.clone(),
                agent("agent-b"),
                vec![agent("reviewer-b")],
                artifact_ref("artifact-a", "hash-a"),
            )
            .unwrap();
        core.submit_output(&node_b, artifact_ref("artifact-b", "hash-b"))
            .unwrap();

        let snapshot = core.get_chain(&chain_id).unwrap();

        assert_eq!(
            snapshot
                .nodes
                .iter()
                .map(|node| node.node_id.clone())
                .collect::<Vec<_>>(),
            vec![root, node_b]
        );
        assert_eq!(
            snapshot
                .artifacts
                .iter()
                .map(|artifact| artifact.artifact_id.clone())
                .collect::<Vec<_>>(),
            vec![artifact_id("artifact-a"), artifact_id("artifact-b")]
        );
        assert_eq!(
            snapshot
                .holders
                .iter()
                .map(|holder| holder.holder_agent.clone())
                .collect::<Vec<_>>(),
            vec![agent("agent-a"), agent("agent-b")]
        );
    }
}
