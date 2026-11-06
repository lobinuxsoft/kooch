//! A `.subgraph`: a graph that generates one function instead of a shader (#1807). A shader reaches
//! it through `// use:`, so saving it changes everything calling it.

use std::path::{Component, Path, PathBuf};

use egui::Pos2;
use egui_snarl::{InPinId, NodeId, OutPinId};
use kooch_render::material::SUBGRAPH_EXTENSION;

use super::{Graph, Node, extract};

/// What the preview says for a subgraph, which is no failure.
pub(crate) const NO_SHAPE: &str =
    "A subgraph has no shape of its own: it shows in the shaders using it.";

/// Whether `path` is a subgraph's file.
pub(crate) fn is_subgraph(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext == SUBGRAPH_EXTENSION)
}

/// `name` as the tail of a WGSL identifier: a pin's name is whatever its author typed.
pub(crate) fn ident(name: &str) -> String {
    name.trim()
        .chars()
        .map(|c| match c.is_ascii_alphanumeric() {
            true => c.to_ascii_lowercase(),
            false => '_',
        })
        .collect()
}

/// The function the subgraph at `path` generates. The prefix keeps a file called `min` from
/// shadowing the builtin.
pub(super) fn function_of(path: &Path) -> String {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    format!("sg_{}", ident(&stem))
}

/// What a new subgraph starts as: a value in, the same value out.
pub(crate) fn starter() -> Graph {
    let mut graph = Graph::new();
    let argument = Node::Argument {
        name: "value".to_owned(),
    };
    let result = Node::Result {
        name: "result".to_owned(),
    };
    let from = graph.insert_node(Pos2::new(40.0, 40.0), argument.into());
    let to = graph.insert_node(Pos2::new(320.0, 40.0), result.into());
    graph.connect(
        OutPinId {
            node: from,
            output: 0,
        },
        InPinId { node: to, input: 0 },
    );
    graph
}

/// The node calling the subgraph held in `source`, named `path` from the shader using it.
pub(crate) fn call_node(file: &Path, source: &str, path: String) -> Option<Node> {
    let graph = extract(source)?;
    let names = |pick: fn(&Node) -> Option<&String>| -> Vec<String> {
        graph
            .node_ids()
            .filter_map(|(_, cell)| pick(&cell.node).cloned())
            .collect()
    };
    let function = function_of(file);
    let reads = ["ui", "surface"].into_iter().find(|kind| {
        let takes = if *kind == "ui" {
            "UiInput"
        } else {
            "SurfaceInput"
        };
        source.contains(&format!("fn {function}(input: {takes}"))
    });
    Some(Node::Subgraph {
        path,
        function,
        inputs: names(|node| match node {
            Node::Argument { name } => Some(name),
            _ => None,
        }),
        outputs: names(|node| match node {
            Node::Result { name } => Some(name),
            _ => None,
        }),
        reads: reads.map(str::to_owned),
    })
}

/// `to` as a `// use:` line in a file inside `folder` names it.
pub(crate) fn relative(folder: &Path, to: &Path) -> String {
    let from: Vec<Component> = folder.components().collect();
    let target: Vec<Component> = to.components().collect();
    let shared = from.iter().zip(&target).take_while(|(a, b)| a == b).count();
    let mut path = PathBuf::new();
    for _ in shared..from.len() {
        path.push("..");
    }
    path.extend(&target[shared..]);
    path.to_string_lossy().replace('\\', "/")
}

/// Brings every subgraph node in `graph`, the graph of `file`, up to its file. Whether one changed.
pub(crate) fn refresh(graph: &mut Graph, file: &Path) -> bool {
    let folder = file.parent().unwrap_or(Path::new(""));
    let calls: Vec<(NodeId, String)> = graph
        .node_ids()
        .filter_map(|(id, cell)| match &cell.node {
            Node::Subgraph { path, .. } => Some((id, path.clone())),
            _ => None,
        })
        .collect();
    let mut changed = false;
    for (id, path) in calls {
        let target = folder.join(&path);
        let fresh = std::fs::read_to_string(&target)
            .ok()
            .and_then(|source| call_node(&target, &source, path));
        let Some(fresh) = fresh else {
            continue;
        };
        if graph.get_node(id).is_some_and(|cell| cell.node != fresh) {
            repin(graph, id, fresh);
            changed = true;
        }
    }
    changed
}

/// Puts `fresh` in `id`. 🔴 Wires and values follow a pin by name: a pin added above the others would
/// otherwise move every wire below it onto the wrong pin, with nothing saying so.
fn repin(graph: &mut Graph, id: NodeId, fresh: Node) {
    let names = |node: &Node| -> (Vec<String>, Vec<String>) {
        let inputs = node.inputs().iter().map(|n| (*n).to_owned()).collect();
        let outputs = node.outputs().iter().map(|o| o.0.to_owned()).collect();
        (inputs, outputs)
    };
    let Some(cell) = graph.get_node(id) else {
        return;
    };
    let (old_in, old_out) = names(&cell.node);
    let (new_in, new_out) = names(&fresh);
    let moved = |index: usize, old: &[String], new: &[String]| {
        let name = old.get(index)?;
        new.iter().position(|n| n == name)
    };
    let wires: Vec<(OutPinId, InPinId)> = graph
        .wires()
        .filter(|(from, to)| from.node == id || to.node == id)
        .collect();
    for (from, to) in &wires {
        graph.disconnect(*from, *to);
    }
    for (mut from, mut to) in wires {
        if from.node == id {
            let Some(output) = moved(from.output, &old_out, &new_out) else {
                continue;
            };
            from.output = output;
        }
        if to.node == id {
            let Some(input) = moved(to.input, &old_in, &new_in) else {
                continue;
            };
            to.input = input;
        }
        graph.connect(from, to);
    }
    if let Some(cell) = graph.get_node_mut(id) {
        let pins = std::mem::take(&mut cell.pins);
        cell.pins = pins
            .into_iter()
            .filter_map(|(input, value)| Some((moved(input, &old_in, &new_in)?, value)))
            .collect();
        cell.node = fresh;
    }
}
