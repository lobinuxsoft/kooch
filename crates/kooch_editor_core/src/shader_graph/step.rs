//! What one frame of editing did to a graph, for its undo history (#1211).

use std::collections::BTreeSet;

use egui_snarl::NodeId;

use super::Graph;
use crate::history::merge::MergeKey;

/// The kind of edit, and which node it touched when that decides what merges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GraphStep {
    AddNodes,
    RemoveNodes,
    Connect,
    Disconnect,
    /// Nodes dragged. A drag is many frames and one step.
    Move,
    /// A value edited on one node. Dragging a number is many frames and one step.
    Edit(NodeId),
    /// A group or a note moved, resized, retitled or retyped.
    Annotate,
}

impl GraphStep {
    /// What the Edit menu calls it.
    pub(crate) fn label(self) -> &'static str {
        match self {
            GraphStep::AddNodes => "Add node",
            GraphStep::RemoveNodes => "Remove node",
            GraphStep::Connect => "Connect",
            GraphStep::Disconnect => "Disconnect",
            GraphStep::Move => "Move nodes",
            GraphStep::Edit(_) => "Edit value",
            GraphStep::Annotate => "Edit group or note",
        }
    }

    /// A continuous edit merges into one step until the mouse is released; a discrete one never
    /// does.
    pub(crate) fn merge_key(self, path: &std::path::Path) -> Option<MergeKey> {
        match self {
            GraphStep::Move => Some(MergeKey::of((path, "move"))),
            GraphStep::Edit(node) => Some(MergeKey::of((path, "edit", node.0))),
            GraphStep::Annotate => Some(MergeKey::of((path, "annotate"))),
            _ => None,
        }
    }
}

/// What changed from `before` to `after`, or `None` when nothing did. Structure wins over values:
/// a frame that added a node and moved it is an addition.
pub(crate) fn change(before: &Graph, after: &Graph) -> Option<GraphStep> {
    let (nodes_before, nodes_after) = (node_set(before), node_set(after));
    if nodes_after.len() > nodes_before.len() {
        return Some(GraphStep::AddNodes);
    }
    if nodes_after != nodes_before {
        return Some(GraphStep::RemoveNodes);
    }
    let (wires_before, wires_after) = (wire_set(before), wire_set(after));
    if wires_after != wires_before {
        return Some(match wires_after.len() >= wires_before.len() {
            true => GraphStep::Connect,
            false => GraphStep::Disconnect,
        });
    }
    let mut moved = false;
    for (id, pos, node) in after.nodes_pos_ids() {
        let Some(old) = before.get_node_info(id) else {
            continue;
        };
        if old.value != *node {
            return Some(GraphStep::Edit(id));
        }
        moved |= old.pos != pos;
    }
    moved.then_some(GraphStep::Move)
}

fn node_set(graph: &Graph) -> BTreeSet<usize> {
    graph.node_ids().map(|(id, _)| id.0).collect()
}

fn wire_set(graph: &Graph) -> BTreeSet<(usize, usize, usize, usize)> {
    graph
        .wires()
        .map(|(out, input)| (out.node.0, out.output, input.node.0, input.input))
        .collect()
}

#[cfg(test)]
mod tests;
