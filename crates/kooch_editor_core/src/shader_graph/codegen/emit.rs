//! One `let` per node, in dependency order: what each node is as a WGSL expression (#1159).

mod noise;
mod uv;

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use egui_snarl::{InPinId, NodeId, OutPinId};

use super::{swizzled, vec4_literal};
use crate::shader_graph::{Graph, Node};

/// The `let` lines, emitted once per node in dependency order.
pub(super) struct Body<'a> {
    pub(super) graph: &'a Graph,
    /// What feeds each input pin.
    /// What feeds each input pin: a node, and which of its outputs.
    pub(super) wires: HashMap<InPinId, OutPinId>,
    pub(super) names: HashMap<NodeId, String>,
    pub(super) visiting: HashSet<NodeId>,
    pub(super) lines: String,
    pub(super) next: u32,
    /// Intermediate `let`s a node writes before its own.
    pub(super) locals: u32,
    /// Whether the body being written is a post-process, which is the only kind with a scene to
    /// read (#1201).
    pub(super) post: bool,
}

impl Body<'_> {
    /// The expression feeding `input` of `node`, emitting whatever it depends on first. An
    /// unconnected input reads as zero: half a graph still renders.
    fn input(&mut self, node: NodeId, input: usize) -> Result<String, String> {
        self.input_or(node, input, "vec4<f32>(0.0)")
    }

    /// A `let` for `expr`, so a value read several times is written once.
    fn local(&mut self, expr: &str) -> String {
        let name = format!("l{}", self.locals);
        self.locals += 1;
        let _ = writeln!(self.lines, "    let {name} = {expr};");
        name
    }

    /// The same, with what an unconnected input reads as.
    pub(super) fn input_or(
        &mut self,
        node: NodeId,
        input: usize,
        fallback: &str,
    ) -> Result<String, String> {
        let pin = InPinId { node, input };
        match self.wires.get(&pin).copied() {
            Some(from) => {
                let value = self.emit(from.node)?;
                let node = self
                    .graph
                    .get_node(from.node)
                    .ok_or("a wire points at a node that is gone")?
                    .clone()
                    .migrated();
                Ok(node.output_of(&value, from.output))
            }
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
            .clone()
            .migrated();
        let mut argument = |body: &mut Self, input: usize| body.input(id, input);
        let value = match &node {
            Node::Uv => "vec4<f32>(input.uv, 0.0, 0.0)".to_owned(),
            Node::SceneColor => {
                let uv = self.input_or(id, 0, "vec4<f32>(input.uv, 0.0, 0.0)")?;
                match self.post {
                    // Only a post-process frame has a scene to read; anywhere else the node is
                    // black rather than a shader that will not compile.
                    true => format!("sample_scene({uv}.xy)"),
                    false => format!("vec4<f32>(0.0) * {uv}.x"),
                }
            }
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
            Node::Constant(value) | Node::ConstColor(value) => vec4_literal(*value),
            Node::ConstFloat(value) => vec4_literal([*value, 0.0, 0.0, 0.0]),
            Node::ConstInt(value) => vec4_literal([value.round(), 0.0, 0.0, 0.0]),
            Node::ConstVector { width, value } => {
                let mut kept = [0.0; 4];
                let wide = (*width).clamp(2, 4) as usize;
                kept[..wide].copy_from_slice(&value[..wide]);
                vec4_literal(kept)
            }
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
            // The value passes through whole; each of its four outputs reads one component.
            Node::Split => argument(self, 0)?,
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
            Node::PolarCoordinates | Node::Twirl | Node::RadialShear | Node::Spherize => {
                self.distortion(id, &node)?
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
            // `octaves` unconnected reads zero, which the helpers take as one: a noise dropped on the
            // canvas is plain noise, and wiring a number in makes it fractal.
            Node::FractalNoise { basis, fractal } => self.fractal_noise(id, basis, fractal)?,
            Node::WhiteNoise => {
                let (uv, scale) = (argument(self, 0)?, argument(self, 1)?);
                format!("vec4<f32>(graph_hash(floor({uv}.xy * {scale}.x)))")
            }
            Node::VoronoiNoise { metric } => self.voronoi(id, metric)?,
            Node::Noise | Node::GradientNoise | Node::SimplexNoise | Node::Voronoi => {
                return Err("an old noise reached emission unmigrated".to_owned());
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
            Node::Output | Node::ShaderOutput { .. } => {
                return Err("the Output node feeds nothing".to_owned());
            }
        };
        self.visiting.remove(&id);
        let name = format!("n{}", self.next);
        self.next += 1;
        let _ = writeln!(self.lines, "    let {name} = {value};");
        self.names.insert(id, name.clone());
        Ok(name)
    }
}
