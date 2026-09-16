//! Auto-arrange (#1167): the graph laid out in layers, from the Surface Output back.
//!
//! Godot's `GraphEditArranger` is the reference: layering, crossing minimisation, horizontal
//! alignment, inner shifts, block placement. A shader graph is a small DAG flowing into one output,
//! so the first two steps carry nearly all of it — but counted **from the end** (user's call): a
//! node's column is how far it is from what it ultimately feeds, so a parameter wired straight into
//! the output sits beside it instead of at the far left with a wire across the whole graph.

use std::collections::HashMap;

use egui::{Pos2, Vec2};
use egui_snarl::NodeId;

use super::{Graph, Node};

/// What Godot leaves between nodes, both ways.
const GAP: Vec2 = Vec2::new(100.0, 100.0);

/// What a node is assumed to occupy. `egui-snarl` never reports the size it drew, and a layout only
/// needs the boxes not to overlap — a generous guess costs empty space, a tight one costs overlap.
const NODE: Vec2 = Vec2::new(240.0, 44.0);

/// What one pin row adds to a node's height.
const ROW: f32 = 28.0;

/// What a node feeds: the node, and which of its input pins.
type Consumers = HashMap<NodeId, Vec<(NodeId, usize)>>;

/// Lays every node out in layers ending at the output, and reports whether anything moved.
///
/// 🔴 A graph that feeds a node into itself is left exactly as it was: there is no longest path to
/// layer it by, and the panel can hold a cycle the codegen would refuse.
pub(crate) fn arrange(graph: &mut Graph) -> bool {
    let Some(ranks) = ranks(graph) else {
        return false;
    };
    let columns = ordered(graph, ranks);
    let last = columns.len().saturating_sub(1);

    let mut moved = false;
    for (rank, column) in columns.iter().enumerate() {
        // Rank 0 is the output's column, drawn furthest right.
        let x = (last - rank) as f32 * (NODE.x + GAP.x);
        let heights: Vec<f32> = column.iter().map(|&id| height(graph, id)).collect();
        let total: f32 = heights.iter().sum::<f32>() + GAP.y * (column.len().max(1) - 1) as f32;
        let mut y = -total / 2.0;
        for (&id, height) in column.iter().zip(&heights) {
            let at = Pos2::new(x, y);
            if let Some(info) = graph.get_node_info_mut(id)
                && info.pos != at
            {
                info.pos = at;
                moved = true;
            }
            y += height + GAP.y;
        }
    }
    moved
}

/// How far each node is from the end of the graph: 0 for a node that feeds nothing — the output, or
/// the end of a dangling branch — and one past the furthest of what it feeds otherwise. `None` on a
/// cycle.
fn ranks(graph: &Graph) -> Option<HashMap<NodeId, usize>> {
    let consumers = consumers(graph);
    let mut ranks = HashMap::new();
    for (id, _, _) in graph.nodes_pos_ids() {
        rank(id, &consumers, &mut ranks, &mut Vec::new())?;
    }
    Some(ranks)
}

fn consumers(graph: &Graph) -> Consumers {
    let mut consumers = Consumers::new();
    for (from, to) in graph.wires() {
        consumers
            .entry(from.node)
            .or_default()
            .push((to.node, to.input));
    }
    consumers
}

fn rank(
    id: NodeId,
    consumers: &Consumers,
    known: &mut HashMap<NodeId, usize>,
    walking: &mut Vec<NodeId>,
) -> Option<usize> {
    if let Some(&rank) = known.get(&id) {
        return Some(rank);
    }
    if walking.contains(&id) {
        return None;
    }
    walking.push(id);
    let mut furthest = 0;
    for &(consumer, _) in consumers.get(&id).into_iter().flatten() {
        furthest = furthest.max(rank(consumer, consumers, known, walking)? + 1);
    }
    walking.pop();
    known.insert(id, furthest);
    Some(furthest)
}

/// The nodes of each column, from the output's back, ordered so wires cross as little as possible.
/// Each node sits at the median of the places it feeds — refined by **which pin**, so what feeds
/// base colour lands above what feeds roughness, in the order the pins are drawn.
fn ordered(graph: &Graph, ranks: HashMap<NodeId, usize>) -> Vec<Vec<NodeId>> {
    let consumers = consumers(graph);
    let deepest = ranks.values().copied().max().unwrap_or(0);
    let mut columns: Vec<Vec<NodeId>> = vec![Vec::new(); deepest + 1];
    for (&id, &rank) in &ranks {
        columns[rank].push(id);
    }
    // The last column feeds nothing: the output first, then any dangling ends, by id so the result
    // is the same every time it runs.
    columns[0].sort_unstable_by_key(|id| {
        let output = graph
            .get_node(*id)
            .is_some_and(|n| matches!(n, Node::Output));
        (!output, id.0)
    });

    let mut places: HashMap<NodeId, f32> = HashMap::new();
    for (place, &id) in columns[0].iter().enumerate() {
        places.insert(id, place as f32);
    }
    for rank in 1..=deepest {
        let mut keyed: Vec<(f32, NodeId)> = columns[rank]
            .iter()
            .map(|&id| (median(graph, &consumers, &places, id), id))
            .collect();
        keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.0.cmp(&b.1.0)));
        columns[rank] = keyed.iter().map(|&(_, id)| id).collect();
        for (place, &id) in columns[rank].iter().enumerate() {
            places.insert(id, place as f32);
        }
    }
    columns
}

/// Where `id` feeds, as a single number: the median of each consumer's place plus how far down its
/// pin list the wire lands.
fn median(graph: &Graph, consumers: &Consumers, places: &HashMap<NodeId, f32>, id: NodeId) -> f32 {
    let mut known: Vec<f32> = consumers
        .get(&id)
        .into_iter()
        .flatten()
        .filter_map(|&(consumer, pin)| {
            let pins = graph
                .get_node(consumer)
                .map_or(1, |n| n.inputs().len().max(1));
            places
                .get(&consumer)
                .map(|place| place + pin as f32 / (pins as f32 + 1.0))
        })
        .collect();
    if known.is_empty() {
        return 0.0;
    }
    known.sort_by(f32::total_cmp);
    known[known.len() / 2]
}

/// What a node is assumed to be tall, by how many pin rows it draws.
fn height(graph: &Graph, id: NodeId) -> f32 {
    let rows = graph
        .get_node(id)
        .map(|node| node.inputs().len().max(usize::from(node.has_output())))
        .unwrap_or(1);
    NODE.y + ROW * rows as f32
}

/// What the panel leaves between a node and the next, for anything that needs to agree with the
/// layout — the minimap's boxes, above all.
pub(crate) const NODE_SIZE: Vec2 = NODE;

#[cfg(test)]
mod tests;
