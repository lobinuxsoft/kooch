//! Graph → `.subgraph`: one function, with a pin per `Argument` and a field per `Result` (#1807).

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use egui_snarl::{InPinId, NodeId, OutPinId};

use super::emit::Body;
use super::helpers::helpers;
use super::{finish, use_lines};
use crate::shader_graph::subgraph::ident;
use crate::shader_graph::{Graph, Node, embed};

/// What a subgraph reads of its shader, from the nodes that only exist in one kind of them.
fn context(graph: &Graph) -> Result<Option<&'static str>, String> {
    let only = |own: &str, other: &str| {
        graph
            .node_ids()
            .any(|(_, node)| node.fits(own) && !node.fits(other))
    };
    match (only("ui", "surface"), only("surface", "ui")) {
        (true, true) => Err("a subgraph cannot mix UI nodes with mesh nodes".to_owned()),
        (true, false) => Ok(Some("ui")),
        (false, true) => Ok(Some("surface")),
        (false, false) => Ok(None),
    }
}

/// The whole `.subgraph` `graph` generates, calling its function `function`.
pub(crate) fn generate_function(graph: &Graph, function: &str) -> Result<String, String> {
    let mut arguments = Vec::new();
    let mut results: Vec<(NodeId, String)> = Vec::new();
    for (id, node) in graph.node_ids() {
        if !node.fits("subgraph") {
            return Err(format!("{} does not exist in a subgraph", node.title()));
        }
        match &node.node {
            Node::Argument { name } => arguments.push(ident(name)),
            Node::Result { name } => results.push((id, ident(name))),
            _ => {}
        }
    }
    if results.is_empty() {
        return Err("the subgraph has no Result node".to_owned());
    }
    let mut seen = HashSet::new();
    let names = arguments.iter().map(|name| format!("in {name}"));
    for name in names.chain(results.iter().map(|(_, name)| format!("out {name}"))) {
        if !seen.insert(name.clone()) {
            return Err(format!("the subgraph has two pins called `{name}`"));
        }
    }

    let wires: HashMap<InPinId, OutPinId> = graph.wires().map(|(from, to)| (to, from)).collect();
    let mut body = Body {
        graph,
        wires,
        names: HashMap::new(),
        visiting: HashSet::new(),
        lines: String::new(),
        next: 0,
        locals: 0,
        post: false,
    };
    let values: Vec<String> = results
        .iter()
        .map(|(id, _)| body.input_or(*id, 0, "vec4<f32>(0.0)"))
        .collect::<Result<_, _>>()?;

    // Uv and Time exist in both kinds, so only what the body wrote says it reads `input` at all.
    let reads_input = [body.lines.as_str()]
        .into_iter()
        .chain(values.iter().map(String::as_str))
        .any(|code| code.contains("input.") || code.contains("(input"));
    let takes = match (context(graph)?, reads_input) {
        (Some("ui"), _) => Some("UiInput"),
        (Some(_), _) | (None, true) => Some("SurfaceInput"),
        (None, false) => None,
    };
    let parameters: Vec<String> = takes
        .map(|kind| format!("input: {kind}"))
        .into_iter()
        .chain(arguments.iter().map(|name| format!("in_{name}: vec4<f32>")))
        .collect();

    let mut code = helpers(graph);
    let _ = writeln!(code, "struct {function}_out {{");
    for (_, name) in &results {
        let _ = writeln!(code, "    out_{name}: vec4<f32>,");
    }
    let _ = writeln!(code, "}}\n");
    let _ = writeln!(
        code,
        "fn {function}({}) -> {function}_out {{",
        parameters.join(", ")
    );
    code.push_str(&body.lines);
    let _ = writeln!(
        code,
        "    return {function}_out({});\n}}",
        values.join(", ")
    );

    let mut source = use_lines(graph);
    let _ = writeln!(source, "{}", embed(graph)?);
    let _ = writeln!(
        source,
        "// Written by the shader graph. Edit it there: saving the graph replaces this file.\n"
    );
    source.push_str(&owned_helpers(&code, function));
    finish(source)
}

/// `code` with the helpers' names made its own: a shader and the subgraphs it uses can each carry
/// the same noise, and a WGSL module cannot declare a name twice.
fn owned_helpers(code: &str, function: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(start) = rest.find(|c: char| c.is_ascii_alphanumeric() || c == '_') {
        let (before, tail) = rest.split_at(start);
        out.push_str(before);
        let end = tail
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(tail.len());
        let (word, after) = tail.split_at(end);
        if ["graph_", "Graph", "GRAPH_"]
            .iter()
            .any(|p| word.starts_with(p))
        {
            let _ = write!(out, "{function}_");
        }
        out.push_str(word);
        rest = after;
    }
    out.push_str(rest);
    out
}
