//! The UV distortions: a coordinate in, a coordinate out, around a centre (#1159).

use egui_snarl::NodeId;

use super::Body;
use crate::shader_graph::Node;

/// Unwired, a distortion reads the mesh's uv around the middle of it: one dropped in does something.
const MESH_UV: &str = "vec4<f32>(input.uv, 0.0, 0.0)";
const MIDDLE: &str = "vec4<f32>(0.5)";

impl Body<'_> {
    pub(super) fn distortion(&mut self, id: NodeId, node: &Node) -> Result<String, String> {
        let uv = self.input_or(id, 0, MESH_UV)?;
        let centre = self.input_or(id, 1, MIDDLE)?;
        // Polar's scales are neutral at one; the others' strength is Unity's default.
        let neutral = match node {
            Node::PolarCoordinates => "vec4<f32>(1.0)",
            _ => "vec4<f32>(10.0)",
        };
        let third = self.input_or(id, 2, neutral)?;
        let fourth = match node {
            Node::PolarCoordinates => self.input_or(id, 3, "vec4<f32>(1.0)")?,
            _ => self.input_or(id, 3, "vec4<f32>(0.0)")?,
        };
        let d = self.local(&format!("{uv}.xy - {centre}.xy"));
        Ok(match node {
            // 🔴 atan2(0, 0) is undefined: nudged off the origin, the centre reads angle 0.5.
            Node::PolarCoordinates => format!(
                "vec4<f32>(length({d}) * 2.0 * {third}.x, \
                 (atan2({d}.x, {d}.y + 0.000000001) / 6.2831855 + 0.5) * {fourth}.x, 0.0, 0.0)"
            ),
            Node::Twirl => {
                let angle = self.local(&format!("{third}.x * length({d})"));
                format!(
                    "vec4<f32>(cos({angle}) * {d}.x - sin({angle}) * {d}.y + {centre}.x + {fourth}.x, \
                     sin({angle}) * {d}.x + cos({angle}) * {d}.y + {centre}.y + {fourth}.y, 0.0, 0.0)"
                )
            }
            Node::RadialShear => format!(
                "vec4<f32>({uv}.xy + vec2<f32>({d}.y, -{d}.x) * dot({d}, {d}) * {third}.x \
                 + {fourth}.xy, 0.0, 0.0)"
            ),
            _ => format!(
                "vec4<f32>({uv}.xy + {d} * dot({d}, {d}) * dot({d}, {d}) * {third}.x \
                 + {fourth}.xy, 0.0, 0.0)"
            ),
        })
    }
}
