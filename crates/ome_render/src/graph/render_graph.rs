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
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Test node that records its name when executed.
    struct RecordNode {
        name: String,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl RenderNode for RecordNode {
        fn name(&self) -> &str {
            &self.name
        }

        fn execute(&mut self, _ctx: &RenderContext<'_>, _encoder: &mut wgpu::CommandEncoder) {
            self.log.lock().unwrap().push(self.name.clone());
        }
    }

    fn record(name: &str, log: &Arc<Mutex<Vec<String>>>) -> RecordNode {
        RecordNode {
            name: name.to_string(),
            log: log.clone(),
        }
    }

    #[test]
    fn empty_graph_orders_to_empty_vec() {
        let graph = RenderGraph::new();
        let order = graph.order().unwrap();
        assert!(order.is_empty());
    }

    #[test]
    fn single_node_orders_alone() {
        let mut graph = RenderGraph::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        graph.add_node(record("alpha", &log));
        assert_eq!(graph.order().unwrap(), vec!["alpha"]);
    }

    #[test]
    fn independent_nodes_keep_insertion_order() {
        let mut graph = RenderGraph::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        graph.add_node(record("first", &log));
        graph.add_node(record("second", &log));
        graph.add_node(record("third", &log));
        assert_eq!(graph.order().unwrap(), vec!["first", "second", "third"]);
    }

    #[test]
    fn dependency_orders_producer_before_consumer() {
        let mut graph = RenderGraph::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        let consumer = graph.add_node(record("consumer", &log));
        let producer = graph.add_node(record("producer", &log));
        graph.connect(producer, consumer).unwrap();
        assert_eq!(graph.order().unwrap(), vec!["producer", "consumer"]);
    }

    #[test]
    fn diamond_dependencies_topo_sort() {
        let mut graph = RenderGraph::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        let root = graph.add_node(record("root", &log));
        let left = graph.add_node(record("left", &log));
        let right = graph.add_node(record("right", &log));
        let join = graph.add_node(record("join", &log));
        graph.connect(root, left).unwrap();
        graph.connect(root, right).unwrap();
        graph.connect(left, join).unwrap();
        graph.connect(right, join).unwrap();

        let order = graph.order().unwrap();
        assert_eq!(order[0], "root");
        assert_eq!(order[3], "join");
        // left/right ordering is up to insertion-order tiebreak; both
        // must come between root and join.
        assert!(order.contains(&"left"));
        assert!(order.contains(&"right"));
    }

    #[test]
    fn cycle_returns_cycle_error() {
        let mut graph = RenderGraph::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        let a = graph.add_node(record("a", &log));
        let b = graph.add_node(record("b", &log));
        graph.connect(a, b).unwrap();
        graph.connect(b, a).unwrap();

        match graph.order() {
            Err(GraphError::Cycle { remaining }) => {
                assert!(remaining.contains(&"a".to_string()));
                assert!(remaining.contains(&"b".to_string()));
            }
            other => panic!("expected Cycle, got {other:?}"),
        }
    }

    #[test]
    fn connect_with_unknown_node_returns_error() {
        let mut graph = RenderGraph::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        let real = graph.add_node(record("real", &log));
        // Build a stale id by removing then trying to use the handle.
        graph.remove_node(real);
        let real2 = graph.add_node(record("real2", &log));
        let err = graph.connect(real, real2).unwrap_err();
        assert!(matches!(err, GraphError::UnknownNode));
    }

    #[test]
    fn duplicate_connect_is_deduped() {
        let mut graph = RenderGraph::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        let a = graph.add_node(record("a", &log));
        let b = graph.add_node(record("b", &log));
        graph.connect(a, b).unwrap();
        graph.connect(a, b).unwrap();
        graph.connect(a, b).unwrap();
        // Topo sort still works.
        assert_eq!(graph.order().unwrap(), vec!["a", "b"]);
    }

    #[test]
    fn remove_node_drops_dependent_edges() {
        let mut graph = RenderGraph::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        let a = graph.add_node(record("a", &log));
        let b = graph.add_node(record("b", &log));
        let c = graph.add_node(record("c", &log));
        graph.connect(a, b).unwrap();
        graph.connect(b, c).unwrap();
        graph.remove_node(b);
        assert_eq!(graph.node_count(), 2);
        assert!(!graph.contains(b));
        // a and c remain — c no longer depends on b (dropped) so order
        // is insertion-based.
        assert_eq!(graph.order().unwrap(), vec!["a", "c"]);
    }

    #[test]
    fn fn_node_records_name() {
        use super::super::node::FnNode;

        let mut graph = RenderGraph::new();
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let log_clone = log.clone();
        graph.add_node(FnNode::new("closure-pass", move |_, _| {
            log_clone.lock().unwrap().push("closure".to_string());
        }));
        assert_eq!(graph.order().unwrap(), vec!["closure-pass"]);
    }
}
