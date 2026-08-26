//! Graph data structure + topological execution.

use std::collections::{HashMap, HashSet, VecDeque};

use super::node::{RenderContext, RenderNode};

slotmap::new_key_type! {
    /// Stable handle to a node in a [`RenderGraph`]. Returned by
    /// [`RenderGraph::add_node`]; passed back to [`RenderGraph::connect`]
    /// when wiring dependencies.
    pub struct NodeId;
}

/// Errors produced when building or executing a graph.
#[derive(Debug)]
pub enum GraphError {
    /// `connect` references a node that no longer lives in the graph.
    UnknownNode,
    /// The dependency graph contains a cycle. Reported names belong to
    /// the nodes still pending after Kahn's algorithm exhausted the
    /// in-degree-zero frontier.
    Cycle { remaining: Vec<String> },
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphError::UnknownNode => {
                write!(f, "render graph: connection references unknown node")
            }
            GraphError::Cycle { remaining } => write!(
                f,
                "render graph: cycle detected, unresolved nodes: {remaining:?}",
            ),
        }
    }
}

impl std::error::Error for GraphError {}

/// Render graph storage + executor.
///
/// `add_node` accepts any `RenderNode` impl. `connect(producer, consumer)`
/// declares "consumer depends on producer" — producer runs first.
/// `execute` topo-sorts the DAG and invokes each node in order on the
/// shared encoder.
pub struct RenderGraph {
    nodes: slotmap::SlotMap<NodeId, GraphEntry>,
    /// `dependencies[node]` = nodes that must run before `node`.
    dependencies: HashMap<NodeId, Vec<NodeId>>,
    /// Insertion order — used by Kahn's algorithm for determinism.
    insertion: Vec<NodeId>,
}

struct GraphEntry {
    node: Box<dyn RenderNode>,
}

impl RenderGraph {
    pub fn new() -> Self {
        Self {
            nodes: slotmap::SlotMap::with_key(),
            dependencies: HashMap::new(),
            insertion: Vec::new(),
        }
    }

    /// Adds a node, returns its handle.
    pub fn add_node(&mut self, node: impl RenderNode) -> NodeId {
        let id = self.nodes.insert(GraphEntry {
            node: Box::new(node),
        });
        self.insertion.push(id);
        id
    }

    /// Removes a node + every dependency edge that referenced it. Stale
    /// `NodeId`s are silently ignored.
    pub fn remove_node(&mut self, id: NodeId) {
        if self.nodes.remove(id).is_none() {
            return;
        }
        self.insertion.retain(|n| *n != id);
        self.dependencies.remove(&id);
        for deps in self.dependencies.values_mut() {
            deps.retain(|n| *n != id);
        }
    }

    /// Declares "`consumer` depends on `producer`" — producer runs
    /// first. Returns [`GraphError::UnknownNode`] if either id is stale.
    /// Duplicate connections are deduped silently.
    pub fn connect(&mut self, producer: NodeId, consumer: NodeId) -> Result<(), GraphError> {
        if !self.nodes.contains_key(producer) || !self.nodes.contains_key(consumer) {
            return Err(GraphError::UnknownNode);
        }
        let deps = self.dependencies.entry(consumer).or_default();
        if !deps.contains(&producer) {
            deps.push(producer);
        }
        Ok(())
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(id)
    }

    /// Topologically sorts the graph (Kahn's algorithm). Stable: ties
    /// broken by insertion order so the same graph always orders the
    /// same way across runs — important for deterministic frame
    /// captures + debugging.
    fn topological_order(&self) -> Result<Vec<NodeId>, GraphError> {
        // Compute in-degree: how many dependencies each node has.
        let mut in_degree: HashMap<NodeId, usize> = self
            .insertion
            .iter()
            .map(|id| {
                let count = self.dependencies.get(id).map_or(0, Vec::len);
                (*id, count)
            })
            .collect();

        let mut frontier: VecDeque<NodeId> = self
            .insertion
            .iter()
            .copied()
            .filter(|id| in_degree[id] == 0)
            .collect();

        let mut visited: HashSet<NodeId> = HashSet::new();
        let mut order: Vec<NodeId> = Vec::with_capacity(self.nodes.len());

        while let Some(id) = frontier.pop_front() {
            if !visited.insert(id) {
                continue;
            }
            order.push(id);
            // Visit consumers (nodes that depend on `id`).
            // dependencies[consumer] contains producers — so we iterate
            // every node and check if it depends on `id`.
            for consumer in self.insertion.iter().copied() {
                if visited.contains(&consumer) {
                    continue;
                }
                let deps = self.dependencies.get(&consumer);
                if deps.is_some_and(|d: &Vec<NodeId>| d.contains(&id)) {
                    let degree = in_degree.entry(consumer).or_insert(0);
                    if *degree > 0 {
                        *degree -= 1;
                    }
                    if *degree == 0 {
                        frontier.push_back(consumer);
                    }
                }
            }
        }

        if order.len() != self.nodes.len() {
            let remaining: Vec<String> = self
                .insertion
                .iter()
                .filter(|id| !visited.contains(id))
                .map(|id| self.nodes[*id].node.name().to_string())
                .collect();
            return Err(GraphError::Cycle { remaining });
        }

        Ok(order)
    }

    /// Resolves the execution order without running anything. Useful for
    /// debugging + tests that verify dependency wiring.
    pub fn order(&self) -> Result<Vec<&str>, GraphError> {
        Ok(self
            .topological_order()?
            .into_iter()
            .map(|id| self.nodes[id].node.name())
            .collect())
    }

    /// Topologically sorts and runs every node on the shared encoder.
    /// Returns [`GraphError::Cycle`] if the graph cannot be linearised.
    pub fn execute(
        &mut self,
        ctx: &RenderContext<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<(), GraphError> {
        let order = self.topological_order()?;
        for id in order {
            // Safe by construction — `id` came from `topological_order`
            // which only emits ids present in `self.nodes`.
            let entry = self
                .nodes
                .get_mut(id)
                .expect("topological_order yielded stale id");
            entry.node.execute(ctx, encoder);
        }
        Ok(())
    }
}

impl Default for RenderGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
