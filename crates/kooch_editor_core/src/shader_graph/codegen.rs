//! Graph → `.shader`: the parameters it declares, one `let` per node, and the graph itself in the
//! block comment that rides above them (#1159).

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use egui_snarl::{InPinId, NodeId};

use super::{Graph, Node, embed};

/// The whole `.shader` a graph generates: header, parameters, body and the graph itself.
pub(crate) fn generate(graph: &Graph) -> Result<String, String> {
    let output = graph
        .node_ids()
        .find(|(_, node)| matches!(node, Node::Output))
        .map(|(id, _)| id)
        .ok_or("the graph has no Surface Output node")?;

    let wires: HashMap<InPinId, NodeId> = graph.wires().map(|(from, to)| (to, from.node)).collect();
    let mut body = Body {
        graph,
        wires,
        names: HashMap::new(),
        visiting: HashSet::new(),
        lines: String::new(),
        next: 0,
    };
    // What an unconnected output falls back to. 🔴 Not zero for the normal: `normalize` of it is
    // NaN, which naga refuses outright.
    let fallbacks = [
        "vec4<f32>(0.0)",
        "vec4<f32>(normalize(input.world_normal), 0.0)",
        "vec4<f32>(0.0)",
        "vec4<f32>(0.5)",
        "vec4<f32>(0.0)",
    ];
    let outputs: Vec<String> = fallbacks
        .iter()
        .enumerate()
        .map(|(input, fallback)| body.input_or(output, input, fallback))
        .collect::<Result<_, _>>()?;

    let mut source = String::new();
    let _ = writeln!(source, "// kind: surface");
    let _ = writeln!(source, "{}", embed(graph)?);
    let _ = writeln!(
        source,
        "// Written by the shader graph. Edit it there: saving the graph replaces this file.\n"
    );
    source.push_str(&declarations(graph));
    let _ = writeln!(
        source,
        "fn surface(input: SurfaceInput) -> SurfaceOutput {{"
    );
    if graph
        .node_ids()
        .any(|(_, n)| matches!(n, Node::Param { .. }))
    {
        let _ = writeln!(source, "    let p = surface_params(input.material_id);");
    }
    source.push_str(&body.lines);
    let _ = writeln!(source, "    var out: SurfaceOutput;");
    let _ = writeln!(source, "    out.base_color = {}.rgb;", outputs[0]);
    let _ = writeln!(source, "    out.normal = normalize({}.xyz);", outputs[1]);
    let _ = writeln!(source, "    out.metallic = {}.x;", outputs[2]);
    let _ = writeln!(source, "    out.roughness = {}.x;", outputs[3]);
    let _ = writeln!(source, "    out.emissive = {}.rgb;", outputs[4]);
    let _ = writeln!(source, "    return out;\n}}");
    Ok(source)
}

/// `struct SurfaceParams`, `SURFACE_DEFAULTS` and the texture declarations the graph's nodes ask
/// for, in the order the nodes were added.
fn declarations(graph: &Graph) -> String {
    let mut out = String::new();
    let params: Vec<&Node> = graph
        .node_ids()
        .map(|(_, node)| node)
        .filter(|node| matches!(node, Node::Param { .. }))
        .collect();
    if !params.is_empty() {
        out.push_str("struct SurfaceParams {\n");
        for node in &params {
            let Node::Param {
                name, width, color, ..
            } = node
            else {
                continue;
            };
            let hint = if *color { "  // @color" } else { "" };
            let _ = writeln!(out, "    {name}: {},{hint}", wgsl_type(*width));
        }
        out.push_str("}\n\nconst SURFACE_DEFAULTS = SurfaceParams(\n");
        for node in &params {
            let Node::Param {
                name,
                width,
                default,
                ..
            } = node
            else {
                continue;
            };
            let _ = writeln!(out, "    {},  // {name}", literal(*width, *default));
        }
        out.push_str(");\n\n");
    }
    for (_, node) in graph.node_ids() {
        if let Node::Texture { name, fallback } = node {
            let _ = writeln!(out, "var {name}: texture_2d<f32>;  // @default({fallback})");
        }
    }
    if graph
        .node_ids()
        .any(|(_, n)| matches!(n, Node::Texture { .. }))
    {
        out.push('\n');
    }
    out
}

fn wgsl_type(width: u32) -> &'static str {
    match width {
        1 => "f32",
        2 => "vec2<f32>",
        3 => "vec3<f32>",
        _ => "vec4<f32>",
    }
}

/// A member's starting value, as WGSL writes it.
fn literal(width: u32, value: [f32; 4]) -> String {
    let numbers: Vec<String> = value
        .iter()
        .take(width.max(1) as usize)
        .map(|v| format!("{v:?}"))
        .collect();
    match width {
        1 => numbers[0].clone(),
        _ => format!("{}({})", wgsl_type(width), numbers.join(", ")),
    }
}

/// The `let` lines, emitted once per node in dependency order.
struct Body<'a> {
    graph: &'a Graph,
    /// What feeds each input pin.
    wires: HashMap<InPinId, NodeId>,
    names: HashMap<NodeId, String>,
    visiting: HashSet<NodeId>,
    lines: String,
    next: u32,
}

impl Body<'_> {
    /// The expression feeding `input` of `node`, emitting whatever it depends on first. An
    /// unconnected input reads as zero: half a graph still renders.
    fn input(&mut self, node: NodeId, input: usize) -> Result<String, String> {
        self.input_or(node, input, "vec4<f32>(0.0)")
    }

    /// The same, with what an unconnected input reads as.
    fn input_or(&mut self, node: NodeId, input: usize, fallback: &str) -> Result<String, String> {
        let pin = InPinId { node, input };
        match self.wires.get(&pin).copied() {
            Some(from) => self.emit(from),
            None => Ok(fallback.to_owned()),
        }
    }

    /// The variable holding `id`'s value.
    fn emit(&mut self, id: NodeId) -> Result<String, String> {
        if let Some(name) = self.names.get(&id) {
            return Ok(name.clone());
        }
        if !self.visiting.insert(id) {
            return Err("the graph feeds a node into itself".to_owned());
        }
        let node = self
            .graph
            .get_node(id)
            .ok_or("a wire points at a node that is gone")?
            .clone();
        let mut argument = |body: &mut Self, input: usize| body.input(id, input);
        let value = match &node {
            Node::Uv => "vec4<f32>(input.uv, 0.0, 0.0)".to_owned(),
            Node::WorldPosition => "vec4<f32>(input.world_position, 1.0)".to_owned(),
            Node::WorldNormal => "vec4<f32>(normalize(input.world_normal), 0.0)".to_owned(),
            Node::ViewDirection => {
                "vec4<f32>(normalize(input.camera_position - input.world_position), 0.0)".to_owned()
            }
            Node::Param { name, width, .. } => match width {
                1 => format!("vec4<f32>(p.{name}, 0.0, 0.0, 0.0)"),
                2 => format!("vec4<f32>(p.{name}, 0.0, 0.0)"),
                3 => format!("vec4<f32>(p.{name}, 0.0)"),
                _ => format!("p.{name}"),
            },
            Node::Constant(value) => format!(
                "vec4<f32>({})",
                value
                    .iter()
                    .map(|v| format!("{v:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Node::Texture { name, .. } => {
                let uv = argument(self, 0)?;
                format!("sample_surface({name}, input, {uv}.xy, vec2<f32>(1.0))")
            }
            Node::Add => {
                let (a, b) = (argument(self, 0)?, argument(self, 1)?);
                format!("{a} + {b}")
            }
            Node::Multiply => {
                let (a, b) = (argument(self, 0)?, argument(self, 1)?);
                format!("{a} * {b}")
            }
            Node::Mix => {
                let (a, b, t) = (argument(self, 0)?, argument(self, 1)?, argument(self, 2)?);
                format!("mix({a}, {b}, vec4<f32>({t}.x))")
            }
            Node::Dot => {
                let (a, b) = (argument(self, 0)?, argument(self, 1)?);
                format!("vec4<f32>(dot({a}.xyz, {b}.xyz))")
            }
            Node::Power => {
                let (a, b) = (argument(self, 0)?, argument(self, 1)?);
                format!("vec4<f32>(pow(max({a}.x, 0.0), {b}.x))")
            }
            Node::Saturate => {
                let value = argument(self, 0)?;
                format!("clamp({value}, vec4<f32>(0.0), vec4<f32>(1.0))")
            }
            Node::Output => return Err("the Surface Output node feeds nothing".to_owned()),
        };
        self.visiting.remove(&id);
        let name = format!("n{}", self.next);
        self.next += 1;
        let _ = writeln!(self.lines, "    let {name} = {value};");
        self.names.insert(id, name.clone());
        Ok(name)
    }
}
