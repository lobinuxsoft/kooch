//! A `.subgraph`: a graph that generates one function instead of a shader (#1807). A shader reaches
//! it through `// use:`, so saving it changes everything calling it.

use std::path::{Component, Path, PathBuf};

use egui::Pos2;
use egui_snarl::{InPinId, NodeId, OutPinId};
use kooch_render::material::SUBGRAPH_EXTENSION;

use kooch_core::shader_uses::engine_source;

use super::{Graph, Node, extract, functions};

/// What the preview says for a subgraph, which is no failure.
pub(crate) const NO_SHAPE: &str =
    "A subgraph has no shape of its own: it shows in the shaders using it.";

/// The Argument a Loop fills with the round.
pub(crate) const INDEX: &str = "index";

/// The Result that ends a Loop.
pub(crate) const STOP: &str = "stop";

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

impl Node {
    /// A subgraph call as a Loop over the same file.
    pub(crate) fn looped(self) -> Self {
        match self {
            Self::Subgraph {
                path,
                function,
                inputs,
                outputs,
                reads,
            } => Self::Loop {
                path,
                function,
                inputs,
                outputs,
                reads,
            },
            other => other,
        }
    }
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

/// The pins of a Loop calling a function with `inputs`: `index` is the loop's to fill.
pub(crate) fn loop_inputs(inputs: &[String]) -> impl Iterator<Item = &str> {
    let own = inputs.iter().map(String::as_str);
    std::iter::once("rounds").chain(own.filter(|name| ident(name) != INDEX))
}

/// The same for what it gives back: `stop` is the loop's to read.
pub(crate) fn loop_outputs(outputs: &[String]) -> impl Iterator<Item = &str> {
    outputs
        .iter()
        .map(String::as_str)
        .filter(|name| ident(name) != STOP)
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
    let calls: Vec<(NodeId, Node)> = graph
        .node_ids()
        .filter(|(_, cell)| {
            matches!(
                cell.node,
                Node::Subgraph { .. } | Node::Loop { .. } | Node::Function { .. }
            )
        })
        .map(|(id, cell)| (id, cell.node.clone()))
        .collect();
    let mut changed = false;
    for (id, node) in calls {
        let Some(fresh) = current(&node, folder) else {
            continue;
        };
        if node != fresh {
            repin(graph, id, fresh);
            changed = true;
        }
    }
    changed
}

/// `node` as the file it calls says it is now; `None` when that file or function is gone, which the
/// shader's own compile reports.
fn current(node: &Node, folder: &Path) -> Option<Node> {
    match node {
        Node::Subgraph { path, .. } | Node::Loop { path, .. } => {
            let target = folder.join(path);
            let source = std::fs::read_to_string(&target).ok()?;
            let fresh = call_node(&target, &source, path.clone())?;
            Some(match node {
                Node::Loop { .. } => fresh.looped(),
                _ => fresh,
            })
        }
        Node::Function { path, function, .. } => {
            let source = match engine_source(path) {
                Some(text) => text.to_owned(),
                None => std::fs::read_to_string(folder.join(path)).ok()?,
            };
            functions(path, &source).into_iter().find_map(|(node, _)| {
                matches!(&node, Node::Function { function: f, .. } if f == function).then_some(node)
            })
        }
        _ => None,
    }
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
