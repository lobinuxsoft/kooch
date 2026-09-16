//! Auto-arrange (#1167): the graph laid out left to right, in layers.
//!
//! Godot's `GraphEditArranger` is the reference. It runs a full Sugiyama pass — layering, crossing
//! minimisation, horizontal alignment, inner shifts, block placement. A shader graph is a small DAG
//! flowing into one Surface Output, so the first two steps carry nearly all of it: layer by longest
//! path, then order each layer by the median of what feeds it.

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

/// Lays every node out in layers, left to right, and reports whether anything moved.
///
/// 🔴 A graph that feeds a node into itself is left exactly as it was: there is no longest path to
/// layer it by, and the panel can hold a cycle the codegen would refuse.
pub(crate) fn arrange(graph: &mut Graph) -> bool {
    let Some(layers) = layers(graph) else {
        return false;
    };
    let ordered = ordered(graph, layers);

    let mut moved = false;
    let mut x = 0.0;
    for layer in &ordered {
        let heights: Vec<f32> = layer.iter().map(|&id| height(graph, id)).collect();
        let total: f32 = heights.iter().sum::<f32>() + GAP.y * (layer.len().max(1) - 1) as f32;
        let mut y = -total / 2.0;
        for (&id, height) in layer.iter().zip(&heights) {
            let at = Pos2::new(x, y);
            if let Some(info) = graph.get_node_info_mut(id)
                && info.pos != at
            {
                info.pos = at;
                moved = true;
            }
            y += height + GAP.y;
        }
        x += NODE.x + GAP.x;
    }
    moved
}

/// Which layer each node belongs to: one past the furthest thing that feeds it. `None` when the
/// graph holds a cycle.
fn layers(graph: &Graph) -> Option<HashMap<NodeId, usize>> {
    let feeders = feeders(graph);
    let mut layers = HashMap::new();
    for (id, _, _) in graph.nodes_pos_ids() {
        depth(id, &feeders, &mut layers, &mut Vec::new())?;
    }
    Some(layers)
}

/// What feeds each node, by node id.
fn feeders(graph: &Graph) -> HashMap<NodeId, Vec<NodeId>> {
    let mut feeders: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for (from, to) in graph.wires() {
        feeders.entry(to.node).or_default().push(from.node);
    }
    feeders
}

/// The longest path from a node with nothing feeding it. `None` on a cycle.
fn depth(
    id: NodeId,
    feeders: &HashMap<NodeId, Vec<NodeId>>,
    known: &mut HashMap<NodeId, usize>,
    walking: &mut Vec<NodeId>,
) -> Option<usize> {
    if let Some(&depth) = known.get(&id) {
        return Some(depth);
    }
    if walking.contains(&id) {
        return None;
    }
    walking.push(id);
    let mut deepest = 0;
    for &feeder in feeders.get(&id).into_iter().flatten() {
        deepest = deepest.max(depth(feeder, feeders, known, walking)? + 1);
    }
    walking.pop();
    known.insert(id, deepest);
    Some(deepest)
}

/// The nodes of each layer, ordered so wires cross as little as possible: each node sits at the
/// median of what feeds it, which is the heuristic Godot's `_crossing_minimisation` splits on.
fn ordered(graph: &Graph, layers: HashMap<NodeId, usize>) -> Vec<Vec<NodeId>> {
    let feeders = feeders(graph);
    let depth = layers.values().copied().max().unwrap_or(0);
    let mut out: Vec<Vec<NodeId>> = vec![Vec::new(); depth + 1];
    for (&id, &layer) in &layers {
        out[layer].push(id);
    }
    // The first layer has nothing feeding it, so its order is whatever it was — sorted by id, which
    // at least makes the result the same every time it runs.
    out[0].sort_unstable_by_key(|id| id.0);

    let mut places: HashMap<NodeId, f32> = HashMap::new();
    for (place, &id) in out[0].iter().enumerate() {
        places.insert(id, place as f32);
    }
    for layer in 1..=depth {
        let mut keyed: Vec<(f32, NodeId)> = out[layer]
            .iter()
            .map(|&id| (median(&feeders, &places, id), id))
            .collect();
        keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.0.cmp(&b.1.0)));
        out[layer] = keyed.iter().map(|&(_, id)| id).collect();
        for (place, &id) in out[layer].iter().enumerate() {
            places.insert(id, place as f32);
        }
    }
    out
}

/// Where the things feeding `id` sit, on average. A node fed by nothing keeps to the top.
fn median(
    feeders: &HashMap<NodeId, Vec<NodeId>>,
    places: &HashMap<NodeId, f32>,
    id: NodeId,
) -> f32 {
    let mut known: Vec<f32> = feeders
        .get(&id)
        .into_iter()
        .flatten()
        .filter_map(|feeder| places.get(feeder).copied())
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
