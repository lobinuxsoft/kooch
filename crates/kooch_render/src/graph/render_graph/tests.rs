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
