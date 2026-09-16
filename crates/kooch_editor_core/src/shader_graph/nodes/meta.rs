//! What each node is called, which pins it has, and which menu it is added from (#1159).

use super::{Category, Node};

impl Node {
    /// What the node is called in the panel.
    pub(crate) fn title(&self) -> String {
        match self {
            Self::Uv => "UV".to_owned(),
            Self::WorldPosition => "World Position".to_owned(),
            Self::WorldNormal => "World Normal".to_owned(),
            Self::ViewDirection => "View Direction".to_owned(),
            Self::Time => "Time".to_owned(),
            Self::Param { name, .. } => format!("Param {name}"),
            Self::Float { name, .. } => format!("Float {name}"),
            Self::Int { name, .. } => format!("Int {name}"),
            Self::Vector { name, width, .. } => format!("Vector {width} {name}"),
            Self::Color { name, .. } => format!("Color {name}"),
            Self::Texture { name, .. } => format!("Texture {name}"),
            Self::Constant(_) => "Constant".to_owned(),
            Self::ConstFloat(_) => "Float".to_owned(),
            Self::ConstInt(_) => "Int".to_owned(),
            Self::ConstVector { width, .. } => format!("Vector {width}"),
            Self::ConstColor(_) => "Color".to_owned(),
            Self::Add => "Add".to_owned(),
            Self::Subtract => "Subtract".to_owned(),
            Self::Multiply => "Multiply".to_owned(),
            Self::Divide => "Divide".to_owned(),
            Self::OneMinus => "One Minus".to_owned(),
            Self::Abs => "Abs".to_owned(),
            Self::Floor => "Floor".to_owned(),
            Self::Fract => "Fract".to_owned(),
            Self::Sine => "Sine".to_owned(),
            Self::Cosine => "Cosine".to_owned(),
            Self::Min => "Min".to_owned(),
            Self::Max => "Max".to_owned(),
            Self::Clamp => "Clamp".to_owned(),
            Self::Step => "Step".to_owned(),
            Self::Smoothstep => "Smoothstep".to_owned(),
            Self::Power => "Power".to_owned(),
            Self::Saturate => "Saturate".to_owned(),
            Self::Remap => "Remap".to_owned(),
            Self::Mix => "Mix".to_owned(),
            Self::Dot => "Dot".to_owned(),
            Self::Cross => "Cross".to_owned(),
            Self::Normalize => "Normalize".to_owned(),
            Self::Length => "Length".to_owned(),
            Self::Distance => "Distance".to_owned(),
            Self::Reflect => "Reflect".to_owned(),
            Self::Swizzle { pattern } => format!("Swizzle {pattern}"),
            Self::Combine => "Combine".to_owned(),
            Self::Split => "Split".to_owned(),
            Self::Fresnel => "Fresnel".to_owned(),
            Self::UnpackNormal => "Unpack Normal".to_owned(),
            Self::Panner => "Panner".to_owned(),
            Self::Rotator => "Rotator".to_owned(),
            Self::Tiling => "Tiling".to_owned(),
            Self::Desaturate => "Desaturate".to_owned(),
            Self::Blend { mode } => format!("Blend {mode}"),
            Self::Noise => "Value Noise".to_owned(),
            Self::GradientNoise => "Gradient Noise".to_owned(),
            Self::SimplexNoise => "Simplex Noise".to_owned(),
            Self::WhiteNoise => "White Noise".to_owned(),
            Self::Voronoi => "Voronoi".to_owned(),
            Self::Circle => "Circle".to_owned(),
            Self::Rectangle => "Rectangle".to_owned(),
            Self::Ring => "Ring".to_owned(),
            Self::Polygon => "Polygon".to_owned(),
            Self::Checker => "Checker".to_owned(),
            Self::Output => "Surface Output".to_owned(),
        }
    }

    /// The inputs it takes, named for their pins.
    pub(crate) fn inputs(&self) -> &'static [&'static str] {
        match self {
            Self::Uv
            | Self::WorldPosition
            | Self::WorldNormal
            | Self::ViewDirection
            | Self::Time
            | Self::Param { .. }
            | Self::Float { .. }
            | Self::Int { .. }
            | Self::Vector { .. }
            | Self::Color { .. }
            | Self::Constant(_)
            | Self::ConstFloat(_)
            | Self::ConstInt(_)
            | Self::ConstVector { .. }
            | Self::ConstColor(_) => &[],
            Self::Texture { .. } => &["uv"],
            Self::Add | Self::Subtract | Self::Multiply | Self::Divide => &["a", "b"],
            Self::OneMinus
            | Self::Abs
            | Self::Floor
            | Self::Fract
            | Self::Sine
            | Self::Cosine
            | Self::Saturate
            | Self::Normalize
            | Self::Length
            | Self::Swizzle { .. } => &["value"],
            Self::Min | Self::Max | Self::Power | Self::Dot | Self::Cross | Self::Distance => {
                &["a", "b"]
            }
            Self::Clamp => &["value", "low", "high"],
            Self::Step => &["edge", "value"],
            Self::Smoothstep => &["low", "high", "value"],
            Self::Remap => &["value", "from", "to"],
            Self::Mix => &["a", "b", "t"],
            Self::Reflect => &["incident", "normal"],
            Self::Combine => &["x", "y", "z", "w"],
            Self::Split => &["value"],
            Self::Fresnel => &["power"],
            Self::UnpackNormal => &["map"],
            Self::Panner => &["uv", "speed"],
            Self::Rotator => &["uv", "centre", "turns"],
            Self::Tiling => &["uv", "tiling", "offset"],
            Self::Desaturate => &["colour", "amount"],
            Self::Blend { .. } => &["a", "b", "opacity"],
            Self::Noise | Self::GradientNoise | Self::SimplexNoise => &["uv", "scale", "octaves"],
            Self::WhiteNoise => &["uv", "scale"],
            Self::Voronoi => &["uv", "scale", "jitter"],
            Self::Circle => &["uv", "radius", "softness"],
            Self::Rectangle => &["uv", "size", "softness"],
            Self::Ring => &["uv", "radius", "thickness"],
            Self::Polygon => &["uv", "sides", "radius"],
            Self::Checker => &["uv", "tiles"],
            Self::Output => &["base color", "normal", "metallic", "roughness", "emissive"],
        }
    }

    /// Whether it produces a value.
    pub(crate) fn has_output(&self) -> bool {
        !self.outputs().is_empty()
    }

    /// Its output pins, named. Most nodes have one, the whole value. A node that answers with several
    /// different things gives each its own pin, rather than packing them into the channels of one
    /// wire where they end up read as a colour.
    pub(crate) fn outputs(&self) -> &'static [&'static str] {
        match self {
            Self::Output => &[],
            Self::Voronoi => &["F1", "F2", "border", "cell"],
            Self::Split => &["x", "y", "z", "w"],
            _ => &["out"],
        }
    }

    /// What output pin `index` reads of the node's value, held in `value`. A single output is the
    /// value itself; one of several is the component that pin names, in every channel.
    pub(crate) fn output_of(&self, value: &str, index: usize) -> String {
        if self.outputs().len() < 2 {
            return value.to_owned();
        }
        let component = ["x", "y", "z", "w"][index.min(3)];
        format!("vec4<f32>({value}.{component})")
    }

    /// Which submenu adds it.
    pub(crate) fn category(&self) -> Category {
        match self {
            Self::Uv
            | Self::WorldPosition
            | Self::WorldNormal
            | Self::ViewDirection
            | Self::Time
            | Self::Param { .. }
            | Self::Float { .. }
            | Self::Int { .. }
            | Self::Vector { .. }
            | Self::Color { .. }
            | Self::Texture { .. }
            | Self::Constant(_) => Category::Input,
            Self::ConstFloat(_)
            | Self::ConstInt(_)
            | Self::ConstVector { .. }
            | Self::ConstColor(_) => Category::Constant,
            Self::Add
            | Self::Subtract
            | Self::Multiply
            | Self::Divide
            | Self::OneMinus
            | Self::Abs
            | Self::Floor
            | Self::Fract
            | Self::Sine
            | Self::Cosine
            | Self::Min
            | Self::Max
            | Self::Clamp
            | Self::Step
            | Self::Smoothstep
            | Self::Power
            | Self::Saturate
            | Self::Remap
            | Self::Mix => Category::Math,
            Self::Dot
            | Self::Cross
            | Self::Normalize
            | Self::Length
            | Self::Distance
            | Self::Reflect
            | Self::Swizzle { .. }
            | Self::Combine
            | Self::Split => Category::Vector,
            Self::Fresnel
            | Self::UnpackNormal
            | Self::Panner
            | Self::Rotator
            | Self::Tiling
            | Self::Desaturate
            | Self::Blend { .. } => Category::Effect,
            Self::Noise
            | Self::GradientNoise
            | Self::SimplexNoise
            | Self::WhiteNoise
            | Self::Voronoi => Category::Noise,
            Self::Circle | Self::Rectangle | Self::Ring | Self::Polygon | Self::Checker => {
                Category::Shape
            }
            Self::Output => Category::Output,
        }
    }

    /// Whether it is one of the noises, which all share `graph_hash`.
    pub(crate) fn is_noise(&self) -> bool {
        matches!(
            self,
            Self::Noise
                | Self::GradientNoise
                | Self::SimplexNoise
                | Self::WhiteNoise
                | Self::Voronoi
        )
    }
}
