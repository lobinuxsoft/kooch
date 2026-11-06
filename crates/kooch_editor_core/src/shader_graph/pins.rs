//! A node and the values typed on its unwired pins (#1809).

use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};

use serde::{Deserialize, Serialize};

use super::Node;

/// What the graph holds per node.
///
/// 🔴 The values live here and not beside the graph: a node's id is reused once it is removed, and
/// a table keyed by id would hand the new node the old one's numbers. Here undo, copy and paste
/// carry them too.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct GraphNode {
    pub(crate) node: Node,
    /// By input pin. A wired pin keeps its value and ignores it, so unwiring brings it back.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) pins: BTreeMap<usize, [f32; 4]>,
}

impl From<Node> for GraphNode {
    fn from(node: Node) -> Self {
        Self {
            node,
            pins: BTreeMap::new(),
        }
    }
}

impl Deref for GraphNode {
    type Target = Node;

    fn deref(&self) -> &Node {
        &self.node
    }
}

impl DerefMut for GraphNode {
    fn deref_mut(&mut self) -> &mut Node {
        &mut self.node
    }
}

impl GraphNode {
    /// What pin `input` reads while unwired, or `None` when it cannot hold a value.
    pub(crate) fn pin_value(&self, input: usize) -> Option<[f32; 4]> {
        let default = self.node.pin_default(input)?;
        Some(self.pins.get(&input).copied().unwrap_or(default))
    }

    /// Types `value` on pin `input`. The default is no value at all, so a noise's optional pins
    /// stop costing their samples when set back to it.
    pub(crate) fn set_pin(&mut self, input: usize, value: [f32; 4]) {
        match self.node.pin_default(input) == Some(value) {
            true => self.pins.remove(&input),
            false => self.pins.insert(input, value),
        };
    }
}

impl Node {
    /// What pin `input` reads unwired, as the codegen's fallbacks say; `None` for a pin whose
    /// fallback is no number, such as this pixel's uv.
    pub(crate) fn pin_default(&self, input: usize) -> Option<[f32; 4]> {
        let name = *self.inputs().get(input)?;
        let splat = |value: f32| Some([value; 4]);
        match (self, name) {
            (Self::Function { .. }, "uv") => None,
            (Self::Function { inputs, .. }, _) => inputs.get(input).map(|pin| pin.default),
            (Self::Loop { .. }, "rounds") => splat(8.0),
            (Self::Subgraph { .. } | Self::Loop { .. }, _) => splat(0.0),
            (_, "uv" | "point" | "map") => None,
            (Self::ShaderOutput { .. } | Self::Output, "normal") => None,
            (Self::ShaderOutput { kind }, "color") if kind == "ui" => None,
            (Self::ShaderOutput { .. } | Self::Output, "alpha") => splat(1.0),
            (Self::ShaderOutput { .. } | Self::Output, "roughness") => splat(0.5),
            (Self::PolarCoordinates, "radial scale" | "length scale") => splat(1.0),
            (
                Self::PolarCoordinates | Self::Twirl | Self::RadialShear | Self::Spherize,
                "centre",
            ) => splat(0.5),
            (Self::Twirl | Self::RadialShear | Self::Spherize, "strength") => splat(10.0),
            (Self::Voronoi | Self::VoronoiNoise { .. }, "randomness") => splat(1.0),
            (_, "scale" | "octaves") if self.is_noise() => splat(1.0),
            (_, "roughness") if self.is_noise() => splat(0.5),
            (_, "lacunarity") if self.is_noise() => splat(2.0),
            (Self::BlinnPhong, "normal" | "light" | "view") => Some([0.0, 0.0, 1.0, 0.0]),
            (Self::BlinnPhong, "shininess") => splat(32.0),
            (Self::SdfFill, "softness") => splat(0.01),
            (Self::SdfStroke, "width") => splat(0.01),
            (Self::SdfStroke, "softness") => splat(0.005),
            (Self::Sway, "speed" | "amount") => splat(1.0),
            (Self::Sway, "ratio") => splat(0.73),
            (Self::LightBand, "direction") => Some([0.8, 0.6, 0.0, 0.0]),
            (Self::LightBand, "width") => splat(0.15),
            (Self::MulAdd, "scale") => splat(1.0),
            _ => splat(0.0),
        }
    }
}
