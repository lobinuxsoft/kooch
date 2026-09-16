//! The shader graph (#1159): what the nodes are, and how a graph is carried inside the `.shader`
//! it generates.
//!
//! 🔴 The graph is the source of truth and the WGSL beside it is output. It rides in a block comment
//! at the top of the file, as Shader Forge carries `/*SF_DATA;…*/`, so the two cannot drift apart
//! and the editor can tell which shaders it may open.

use egui_snarl::Snarl;

mod arrange;
mod codegen;
mod nodes;

pub(crate) use arrange::{NODE_SIZE, arrange};
pub(crate) use codegen::generate;
pub(crate) use nodes::{BLEND_MODES, Category, Node, TEXTURE_FALLBACKS, palette};

/// What the block comment holding a graph starts with.
const MARKER: &str = "/*KOOCH_GRAPH;";

/// A graph and where its nodes sit, as the panel holds it.
pub(crate) type Graph = Snarl<Node>;

/// Nodes **and where they sit**, for telling whether the panel changed anything.
///
/// 🔴 Dragging a node is an edit the file has to keep: comparing only the values let a whole
/// re-layout be lost on close without a word about it (#1167).
pub(crate) fn snapshot(graph: &Graph) -> Vec<(egui::Pos2, Node)> {
    graph
        .nodes_pos_ids()
        .map(|(_, pos, node)| (pos, node.clone()))
        .collect()
}

/// The graph carried by `source`, if it holds one.
pub(crate) fn extract(source: &str) -> Option<Graph> {
    let start = source.find(MARKER)? + MARKER.len();
    let end = start + source[start..].find("*/")?;
    match ron::from_str(&source[start..end]) {
        Ok(graph) => Some(graph),
        Err(error) => {
            tracing::warn!(%error, "a shader carries a graph this editor cannot read");
            None
        }
    }
}

/// Whether `source` was written by the graph tool — what decides if the editor offers to open it.
pub(crate) fn is_generated(source: &str) -> bool {
    source.contains(MARKER)
}

/// The block that carries `graph`, as one line so the file stays readable below it.
fn embed(graph: &Graph) -> Result<String, String> {
    let ron = ron::ser::to_string(graph).map_err(|e| e.to_string())?;
    // 🔴 A `*/` inside the data would close the comment early. Nothing serialises one today, and a
    // graph that did would silently truncate rather than fail.
    if ron.contains("*/") {
        return Err("the graph holds `*/`, which would close its own comment".to_owned());
    }
    Ok(format!("{MARKER}{ron}*/"))
}

/// What a new graph starts as: an albedo texture tinted by a colour, with roughness of its own —
/// the shape of a material, so a fresh graph already renders like one.
pub(crate) fn starter() -> Graph {
    use egui::Pos2;
    use egui_snarl::{InPinId, OutPinId};

    let mut graph = Graph::new();
    let uv = graph.insert_node(Pos2::new(40.0, 40.0), Node::Uv);
    let albedo = graph.insert_node(
        Pos2::new(220.0, 40.0),
        Node::Texture {
            name: "albedo".to_owned(),
            fallback: "white".to_owned(),
        },
    );
    let tint = graph.insert_node(
        Pos2::new(220.0, 200.0),
        Node::Param {
            name: "base_color".to_owned(),
            width: 4,
            color: true,
            default: [1.0; 4],
        },
    );
    let roughness = graph.insert_node(
        Pos2::new(220.0, 320.0),
        Node::Param {
            name: "roughness".to_owned(),
            width: 1,
            color: false,
            default: [0.5, 0.0, 0.0, 0.0],
        },
    );
    let tinted = graph.insert_node(Pos2::new(430.0, 100.0), Node::Multiply);
    let output = graph.insert_node(Pos2::new(640.0, 140.0), Node::Output);
    let mut wire = |from, to, input| {
        graph.connect(
            OutPinId {
                node: from,
                output: 0,
            },
            InPinId { node: to, input },
        );
    };
    wire(uv, albedo, 0);
    wire(albedo, tinted, 0);
    wire(tint, tinted, 1);
    wire(tinted, output, 0);
    wire(roughness, output, 3);
    graph
}

#[cfg(test)]
mod tests;
