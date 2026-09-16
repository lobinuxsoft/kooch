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
    source.push_str(&helpers(graph));
    let _ = writeln!(
        source,
        "fn surface(input: SurfaceInput) -> SurfaceOutput {{"
    );
    if graph.node_ids().any(|(_, n)| n.declared().is_some()) {
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
    // 🔴 The graph never writes a file the engine cannot read. A shader that fails to parse is
    // written all the same, fails to reload, and leaves every reader — the Inspector above all —
    // showing the parameters from before it, with nothing on screen saying why (#1159).
    if let Err(error) = kooch_render::material::Shader::parse(&source) {
        return Err(format!("{error}"));
    }
    Ok(source)
}

/// `struct SurfaceParams`, `SURFACE_DEFAULTS` and the texture declarations the graph's nodes ask
/// for, in the order the nodes were added.
fn declarations(graph: &Graph) -> String {
    let mut out = String::new();
    let params: Vec<_> = graph
        .node_ids()
        .filter_map(|(_, node)| node.declared())
        .collect();
    if !params.is_empty() {
        out.push_str("struct SurfaceParams {\n");
        for param in &params {
            let hint = match param.hint.as_str() {
                "" => String::new(),
                hint => format!("  // {hint}"),
            };
            let _ = writeln!(out, "    {}: {},{hint}", param.name, wgsl_type(param.width));
        }
        out.push_str("}\n\nconst SURFACE_DEFAULTS = SurfaceParams(\n");
        for param in &params {
            let _ = writeln!(
                out,
                "    {},  // {}",
                literal(param.width, param.default),
                param.name
            );
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

/// `graph_rotate`, `graph_noise` and the shape helpers, each emitted only when a node in the graph
/// asks for it: a generated file carries nothing it does not use.
fn helpers(graph: &Graph) -> String {
    let uses = |wanted: fn(&Node) -> bool| graph.node_ids().any(|(_, node)| wanted(node));
    let mut out = String::new();
    for (wanted, body) in [
        (
            (|n| matches!(n, Node::Rotator)) as fn(&Node) -> bool,
            ROTATE_HELPER,
        ),
        (
            |n| matches!(n, Node::Normalize | Node::Reflect | Node::UnpackNormal),
            NORMAL_HELPER,
        ),
        (|n| matches!(n, Node::Blend { .. }), OVERLAY_HELPER),
        (|n| matches!(n, Node::Noise), NOISE_HELPER),
        (|n| matches!(n, Node::Rectangle), BOX_HELPER),
        (|n| matches!(n, Node::Ring), RING_HELPER),
        (|n| matches!(n, Node::Polygon), POLYGON_HELPER),
        (|n| matches!(n, Node::Checker), CHECKER_HELPER),
    ] {
        if uses(wanted) {
            out.push_str(body);
            out.push('\n');
        }
    }
    out
}

/// Turns a coordinate around a centre.
const ROTATE_HELPER: &str = "\
fn graph_rotate(uv: vec2<f32>, centre: vec2<f32>, angle: f32) -> vec2<f32> {
    let d = uv - centre;
    let s = sin(angle);
    let c = cos(angle);
    return centre + vec2<f32>(d.x * c - d.y * s, d.x * s + d.y * c);
}
";

/// 🔴 `normalize` of a zero vector is NaN, and a graph reaches one the moment an input is left
/// unconnected. Up is the answer that keeps rendering.
const NORMAL_HELPER: &str = "\
fn graph_normalize(v: vec3<f32>) -> vec3<f32> {
    return select(normalize(v), vec3<f32>(0.0, 0.0, 1.0), length(v) < 0.000001);
}
";

/// The one blend mode that is not a one-liner.
const OVERLAY_HELPER: &str = "\
fn graph_overlay(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    let low = 2.0 * a * b;
    let high = vec4<f32>(1.0) - 2.0 * (vec4<f32>(1.0) - a) * (vec4<f32>(1.0) - b);
    return select(low, high, a > vec4<f32>(0.5));
}
";

/// Value noise: a hash at each cell corner, smoothed between them.
const NOISE_HELPER: &str = "\
fn graph_hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.547);
}

fn graph_noise(uv: vec2<f32>) -> f32 {
    let cell = floor(uv);
    let f = fract(uv);
    let a = graph_hash(cell);
    let b = graph_hash(cell + vec2<f32>(1.0, 0.0));
    let c = graph_hash(cell + vec2<f32>(0.0, 1.0));
    let d = graph_hash(cell + vec2<f32>(1.0, 1.0));
    let w = f * f * (3.0 - 2.0 * f);
    return mix(mix(a, b, w.x), mix(c, d, w.x), w.y);
}
";

const BOX_HELPER: &str = "\
fn graph_box(uv: vec2<f32>, size: vec2<f32>, softness: f32) -> f32 {
    let d = abs(uv - vec2<f32>(0.5)) - size * 0.5;
    let s = max(softness, 0.00001);
    let m = vec2<f32>(1.0) - smoothstep(vec2<f32>(-s), vec2<f32>(s), d);
    return m.x * m.y;
}
";

const RING_HELPER: &str = "\
fn graph_ring(uv: vec2<f32>, radius: f32, thickness: f32) -> f32 {
    let d = abs(length(uv - vec2<f32>(0.5)) - radius);
    let t = max(thickness, 0.00001) * 0.5;
    return 1.0 - smoothstep(t * 0.8, t, d);
}
";

const POLYGON_HELPER: &str = "\
fn graph_polygon(uv: vec2<f32>, sides: f32, radius: f32) -> f32 {
    let p = uv - vec2<f32>(0.5);
    let n = max(floor(sides + 0.5), 3.0);
    let segment = 6.2831855 / n;
    let a = atan2(p.y, p.x + 0.000000001);
    let d = length(p) * cos(a - segment * floor(a / segment + 0.5)) / cos(segment * 0.5);
    return 1.0 - smoothstep(radius - 0.005, radius + 0.005, d);
}
";

const CHECKER_HELPER: &str = "\
fn graph_checker(uv: vec2<f32>, tiles: vec2<f32>) -> f32 {
    let cell = floor(uv * tiles);
    return fract((cell.x + cell.y) * 0.5) * 2.0;
}
";

/// A swizzle as WGSL, widened to four components. A pattern naming anything but `x`, `y`, `z` or `w`
/// is refused rather than written: the file would fail to parse with the graph's own name on it.
fn swizzled(value: &str, pattern: &str) -> Result<String, String> {
    let pattern = pattern.trim();
    if pattern.is_empty() || pattern.len() > 4 || !pattern.chars().all(|c| "xyzw".contains(c)) {
        return Err(format!("`{pattern}` is not a swizzle of x, y, z or w"));
    }
    Ok(match pattern.len() {
        1 => format!("vec4<f32>({value}.{pattern})"),
        2 => format!("vec4<f32>({value}.{pattern}, 0.0, 0.0)"),
        3 => format!("vec4<f32>({value}.{pattern}, 0.0)"),
        _ => format!("{value}.{pattern}"),
    })
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
            Node::Param { .. }
            | Node::Float { .. }
            | Node::Int { .. }
            | Node::Vector { .. }
            | Node::Color { .. } => {
                let param = node.declared().ok_or("a parameter node declares nothing")?;
                let name = param.name;
                match param.width {
                    1 => format!("vec4<f32>(p.{name}, 0.0, 0.0, 0.0)"),
                    2 => format!("vec4<f32>(p.{name}, 0.0, 0.0)"),
                    3 => format!("vec4<f32>(p.{name}, 0.0)"),
                    _ => format!("p.{name}"),
                }
            }
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
            Node::Time => {
                "vec4<f32>(input.time, sin(input.time), cos(input.time), input.time * 0.1)"
                    .to_owned()
            }
            Node::Subtract => {
                let (a, b) = (argument(self, 0)?, argument(self, 1)?);
                format!("{a} - {b}")
            }
            // 🔴 `sign` is what makes this safe: at zero it is zero, so the result is zero rather
            // than an infinity that spreads through everything downstream of it.
            Node::Divide => {
                let (a, b) = (argument(self, 0)?, argument(self, 1)?);
                format!("{a} / max(abs({b}), vec4<f32>(0.00001)) * sign({b})")
            }
            Node::OneMinus => {
                let value = argument(self, 0)?;
                format!("vec4<f32>(1.0) - {value}")
            }
            Node::Abs => format!("abs({})", argument(self, 0)?),
            Node::Floor => format!("floor({})", argument(self, 0)?),
            Node::Fract => format!("fract({})", argument(self, 0)?),
            Node::Sine => format!("sin({})", argument(self, 0)?),
            Node::Cosine => format!("cos({})", argument(self, 0)?),
            Node::Min => {
                let (a, b) = (argument(self, 0)?, argument(self, 1)?);
                format!("min({a}, {b})")
            }
            Node::Max => {
                let (a, b) = (argument(self, 0)?, argument(self, 1)?);
                format!("max({a}, {b})")
            }
            Node::Clamp => {
                let (value, low, high) =
                    (argument(self, 0)?, argument(self, 1)?, argument(self, 2)?);
                format!("clamp({value}, vec4<f32>({low}.x), vec4<f32>(max({high}.x, {low}.x)))")
            }
            Node::Step => {
                let (edge, value) = (argument(self, 0)?, argument(self, 1)?);
                format!("step(vec4<f32>({edge}.x), {value})")
            }
            // 🔴 Two equal edges make `smoothstep` divide by zero, and an unconnected pair is two
            // zeroes. The high edge is held above the low one.
            Node::Smoothstep => {
                let (low, high, value) =
                    (argument(self, 0)?, argument(self, 1)?, argument(self, 2)?);
                format!(
                    "smoothstep(vec4<f32>({low}.x), vec4<f32>(max({high}.x, {low}.x + 0.00001)), \
                     {value})"
                )
            }
            Node::Remap => {
                let (value, from, to) =
                    (argument(self, 0)?, argument(self, 1)?, argument(self, 2)?);
                format!(
                    "vec4<f32>({to}.x) + ({value} - vec4<f32>({from}.x)) * \
                     vec4<f32>(({to}.y - {to}.x) / max(abs({from}.y - {from}.x), 0.00001))"
                )
            }
            Node::Cross => {
                let (a, b) = (argument(self, 0)?, argument(self, 1)?);
                format!("vec4<f32>(cross({a}.xyz, {b}.xyz), 0.0)")
            }
            Node::Normalize => {
                let value = argument(self, 0)?;
                format!("vec4<f32>(graph_normalize({value}.xyz), {value}.w)")
            }
            Node::Length => format!("vec4<f32>(length({}.xyz))", argument(self, 0)?),
            Node::Distance => {
                let (a, b) = (argument(self, 0)?, argument(self, 1)?);
                format!("vec4<f32>(distance({a}.xyz, {b}.xyz))")
            }
            Node::Reflect => {
                let (incident, normal) = (argument(self, 0)?, argument(self, 1)?);
                format!("vec4<f32>(reflect({incident}.xyz, graph_normalize({normal}.xyz)), 0.0)")
            }
            Node::Swizzle { pattern } => {
                let value = argument(self, 0)?;
                swizzled(&value, pattern)?
            }
            Node::Combine => {
                let (x, y, z, w) = (
                    argument(self, 0)?,
                    argument(self, 1)?,
                    argument(self, 2)?,
                    argument(self, 3)?,
                );
                format!("vec4<f32>({x}.x, {y}.x, {z}.x, {w}.x)")
            }
            Node::Fresnel => {
                let power = argument(self, 0)?;
                format!(
                    "vec4<f32>(pow(1.0 - clamp(dot(normalize(input.world_normal), \
                     normalize(input.camera_position - input.world_position)), 0.0, 1.0), \
                     max({power}.x, 0.0001)))"
                )
            }
            // The mesh's tangent frame, built exactly as the engine's own surface builds it.
            Node::UnpackNormal => {
                let map = argument(self, 0)?;
                format!(
                    "vec4<f32>(graph_normalize(mat3x3<f32>(normalize(input.world_tangent.xyz), \
                     cross(normalize(input.world_normal), normalize(input.world_tangent.xyz)) * \
                     input.world_tangent.w, normalize(input.world_normal)) * \
                     ({map}.xyz * 2.0 - 1.0)), 0.0)"
                )
            }
            Node::Panner => {
                let (uv, speed) = (argument(self, 0)?, argument(self, 1)?);
                format!("vec4<f32>({uv}.xy + {speed}.xy * input.time, 0.0, 0.0)")
            }
            Node::Rotator => {
                let (uv, centre, turns) =
                    (argument(self, 0)?, argument(self, 1)?, argument(self, 2)?);
                format!(
                    "vec4<f32>(graph_rotate({uv}.xy, {centre}.xy, {turns}.x * 6.2831855), 0.0, 0.0)"
                )
            }
            Node::Tiling => {
                let (uv, tiling, offset) =
                    (argument(self, 0)?, argument(self, 1)?, argument(self, 2)?);
                format!("vec4<f32>({uv}.xy * {tiling}.xy + {offset}.xy, 0.0, 0.0)")
            }
            Node::Desaturate => {
                let (colour, amount) = (argument(self, 0)?, argument(self, 1)?);
                format!(
                    "mix({colour}, vec4<f32>(vec3<f32>(dot({colour}.rgb, \
                     vec3<f32>(0.2126, 0.7152, 0.0722))), {colour}.a), \
                     vec4<f32>(clamp({amount}.x, 0.0, 1.0)))"
                )
            }
            Node::Blend { mode } => {
                let (a, b, opacity) = (argument(self, 0)?, argument(self, 1)?, argument(self, 2)?);
                let blended = match mode.as_str() {
                    "multiply" => format!("{a} * {b}"),
                    "screen" => {
                        format!("vec4<f32>(1.0) - (vec4<f32>(1.0) - {a}) * (vec4<f32>(1.0) - {b})")
                    }
                    "overlay" => format!("graph_overlay({a}, {b})"),
                    "lighten" => format!("max({a}, {b})"),
                    "darken" => format!("min({a}, {b})"),
                    other => return Err(format!("`{other}` is not a blend mode")),
                };
                format!("mix({a}, {blended}, vec4<f32>(clamp({opacity}.x, 0.0, 1.0)))")
            }
            Node::Noise => {
                let (uv, scale) = (argument(self, 0)?, argument(self, 1)?);
                format!("vec4<f32>(graph_noise({uv}.xy * {scale}.x))")
            }
            Node::Circle => {
                let (uv, radius, softness) =
                    (argument(self, 0)?, argument(self, 1)?, argument(self, 2)?);
                format!(
                    "vec4<f32>(1.0 - smoothstep({radius}.x - max({softness}.x, 0.00001), \
                     {radius}.x + max({softness}.x, 0.00001), \
                     length({uv}.xy - vec2<f32>(0.5))))"
                )
            }
            Node::Rectangle => {
                let (uv, size, softness) =
                    (argument(self, 0)?, argument(self, 1)?, argument(self, 2)?);
                format!("vec4<f32>(graph_box({uv}.xy, {size}.xy, {softness}.x))")
            }
            Node::Ring => {
                let (uv, radius, thickness) =
                    (argument(self, 0)?, argument(self, 1)?, argument(self, 2)?);
                format!("vec4<f32>(graph_ring({uv}.xy, {radius}.x, {thickness}.x))")
            }
            Node::Polygon => {
                let (uv, sides, radius) =
                    (argument(self, 0)?, argument(self, 1)?, argument(self, 2)?);
                format!("vec4<f32>(graph_polygon({uv}.xy, {sides}.x, {radius}.x))")
            }
            Node::Checker => {
                let (uv, tiles) = (argument(self, 0)?, argument(self, 1)?);
                format!("vec4<f32>(graph_checker({uv}.xy, {tiles}.xy))")
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
