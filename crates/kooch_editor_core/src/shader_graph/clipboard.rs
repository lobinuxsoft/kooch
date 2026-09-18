//! Copying nodes between places in a graph, or between graphs (#1211).

use std::collections::{BTreeSet, HashMap};

use egui::{Pos2, Vec2};
use egui_snarl::{InPinId, NodeId, OutPinId};

use super::{Graph, Node};

/// Nodes and the wires between them, positioned relative to their top-left corner.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Clip {
    nodes: Vec<(Vec2, Node)>,
    /// `(from node, output, to node, input)`, the nodes as indices into `nodes`.
    wires: Vec<(usize, usize, usize, usize)>,
}

/// The selected nodes and the wires running between two of them. The output node stays: a graph
/// has one. `None` when nothing copyable is selected.
pub(crate) fn copy(graph: &Graph, selected: &[NodeId]) -> Option<Clip> {
    let picked: Vec<(NodeId, Pos2, Node)> = selected
        .iter()
        .filter_map(|&id| {
            let info = graph.get_node_info(id)?;
            (info.value.output_kind().is_none()).then(|| (id, info.pos, info.value.clone()))
        })
        .collect();
    let corner = picked
        .iter()
        .map(|(_, pos, _)| *pos)
        .reduce(|a, b| a.min(b))?;
    let index: HashMap<NodeId, usize> = picked
        .iter()
        .enumerate()
        .map(|(at, (id, ..))| (*id, at))
        .collect();
    let wires = graph
        .wires()
        .filter_map(|(out, input)| {
            Some((
                *index.get(&out.node)?,
                out.output,
                *index.get(&input.node)?,
                input.input,
            ))
        })
        .collect();
    Some(Clip {
        nodes: picked
            .into_iter()
            .map(|(_, pos, node)| (pos - corner, node))
            .collect(),
        wires,
    })
}

/// Places `clip` with its top-left corner at `at`, returning the new nodes. A parameter whose name
/// is taken is renamed: two parameters of one name are one uniform, and the second would be lost.
pub(crate) fn paste(graph: &mut Graph, clip: &Clip, at: Pos2) -> Vec<NodeId> {
    let mut taken: BTreeSet<String> = graph
        .node_ids()
        .filter_map(|(_, node)| Some(node.declared()?.name.to_owned()))
        .collect();
    let placed: Vec<NodeId> = clip
        .nodes
        .iter()
        .map(|(offset, node)| {
            let mut node = node.clone();
            if let Some(name) = name_mut(&mut node) {
                *name = free_name(name, &taken);
                taken.insert(name.clone());
            }
            graph.insert_node(at + *offset, node)
        })
        .collect();
    for &(from, output, to, input) in &clip.wires {
        graph.connect(
            OutPinId {
                node: placed[from],
                output,
            },
            InPinId {
                node: placed[to],
                input,
            },
        );
    }
    placed
}

/// Removes `selected`, and the wires into and out of them with them.
pub(crate) fn remove(graph: &mut Graph, selected: &[NodeId]) {
    for &id in selected {
        if graph.get_node(id).is_some() {
            graph.remove_node(id);
        }
    }
}

/// Where the clip's top-left corner sat in `graph` when it was copied from `selected` — what a
/// duplicate is offset from.
pub(crate) fn corner(graph: &Graph, selected: &[NodeId]) -> Option<Pos2> {
    selected
        .iter()
        .filter_map(|&id| Some(graph.get_node_info(id)?.pos))
        .reduce(|a, b| a.min(b))
}

/// The name a parameter node declares, for renaming it.
fn name_mut(node: &mut Node) -> Option<&mut String> {
    match node {
        Node::Param { name, .. }
        | Node::Float { name, .. }
        | Node::Int { name, .. }
        | Node::Vector { name, .. }
        | Node::Color { name, .. }
        | Node::Texture { name, .. } => Some(name),
        _ => None,
    }
}

/// `name`, or `name_2`, `name_3`… — the first that `taken` does not hold.
fn free_name(name: &str, taken: &BTreeSet<String>) -> String {
    if !taken.contains(name) {
        return name.to_owned();
    }
    (2..)
        .map(|n| format!("{name}_{n}"))
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or_else(|| name.to_owned())
}

#[cfg(test)]
mod tests;
