//! What each node is called, which pins it has, and which menu it is added from (#1159).

use super::{Category, Node};

/// What an output pin reads of its node's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    /// All four components, as the node produced them.
    Whole,
    /// The first three, for a colour without its alpha.
    Rgb,
    /// One component, in every channel of the wire.
    Channel(usize),
    /// The last three: the colour a fractal noise packs after its value.
    Yzw,
    /// A named field of a node whose value is a struct, in every channel.
    Field(&'static str),
    /// A two-wide named field of such a struct.
    FieldXy(&'static str),
}

const WHOLE: &[(&str, Pick)] = &[("out", Pick::Whole)];
const RGBA: &[(&str, Pick)] = &[
    ("RGBA", Pick::Whole),
    ("RGB", Pick::Rgb),
    ("R", Pick::Channel(0)),
    ("G", Pick::Channel(1)),
    ("B", Pick::Channel(2)),
    ("A", Pick::Channel(3)),
];
const XYZ: &[(&str, Pick)] = &[
    ("XYZ", Pick::Whole),
    ("X", Pick::Channel(0)),
    ("Y", Pick::Channel(1)),
    ("Z", Pick::Channel(2)),
];
const XYZW: &[(&str, Pick)] = &[
    ("XYZW", Pick::Whole),
    ("X", Pick::Channel(0)),
    ("Y", Pick::Channel(1)),
    ("Z", Pick::Channel(2)),
    ("W", Pick::Channel(3)),
];
const XY: &[(&str, Pick)] = &[
    ("XY", Pick::Whole),
    ("X", Pick::Channel(0)),
    ("Y", Pick::Channel(1)),
];
const UV: &[(&str, Pick)] = &[
    ("UV", Pick::Whole),
    ("U", Pick::Channel(0)),
    ("V", Pick::Channel(1)),
];
const POLAR: &[(&str, Pick)] = &[
    ("polar", Pick::Whole),
    ("radius", Pick::Channel(0)),
    ("angle", Pick::Channel(1)),
];
const TIME: &[(&str, Pick)] = &[
    ("time", Pick::Channel(0)),
    ("sine", Pick::Channel(1)),
    ("cosine", Pick::Channel(2)),
    ("tenth", Pick::Channel(3)),
];
const VORONOI: &[(&str, Pick)] = &[
    ("F1", Pick::Field("f1")),
    ("F2", Pick::Field("f2")),
    ("border", Pick::Field("edge")),
    ("cell", Pick::Field("cell")),
    ("position", Pick::FieldXy("position")),
];
/// A fractal noise packs its value in `x` and, when the colour pin is wired, a colour in `yzw`.
const FRACTAL: &[(&str, Pick)] = &[("value", Pick::Channel(0)), ("color", Pick::Yzw)];
const SPLIT: &[(&str, Pick)] = &[
    ("x", Pick::Channel(0)),
    ("y", Pick::Channel(1)),
    ("z", Pick::Channel(2)),
    ("w", Pick::Channel(3)),
];

impl Node {
    /// What the node is called in the panel.
    pub(crate) fn title(&self) -> String {
        match self {
            Self::Uv => "UV".to_owned(),
            Self::SceneColor => "Scene Color".to_owned(),
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
            Self::PolarCoordinates => "Polar Coordinates".to_owned(),
            Self::Twirl => "Twirl".to_owned(),
            Self::RadialShear => "Radial Shear".to_owned(),
            Self::Spherize => "Spherize".to_owned(),
            Self::Desaturate => "Desaturate".to_owned(),
            Self::Blend { mode } => format!("Blend {mode}"),
            Self::FractalNoise { basis, .. } => {
                let mut title = basis.clone();
                if let Some(first) = title.get_mut(0..1) {
                    first.make_ascii_uppercase();
                }
                format!("{title} Noise")
            }
            Self::Noise => "Value Noise".to_owned(),
            Self::GradientNoise => "Gradient Noise".to_owned(),
            Self::SimplexNoise => "Simplex Noise".to_owned(),
            Self::WhiteNoise => "White Noise".to_owned(),
            Self::Voronoi | Self::VoronoiNoise { .. } => "Voronoi".to_owned(),
            Self::Circle => "Circle".to_owned(),
            Self::Rectangle => "Rectangle".to_owned(),
            Self::Ring => "Ring".to_owned(),
            Self::Polygon => "Polygon".to_owned(),
            Self::Checker => "Checker".to_owned(),
            Self::Output => "Surface Output".to_owned(),
            Self::ShaderOutput { kind } => {
                let mut title = kind.clone();
                if let Some(first) = title.get_mut(0..1) {
                    first.make_ascii_uppercase();
                }
                format!("{title} Output")
            }
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
            Self::PolarCoordinates => &["uv", "centre", "radial scale", "length scale"],
            Self::Twirl | Self::RadialShear | Self::Spherize => {
                &["uv", "centre", "strength", "offset"]
            }
            Self::Desaturate => &["colour", "amount"],
            Self::Blend { .. } => &["a", "b", "opacity"],
            // 🔴 In this order: pins are wired by index, and an old noise's uv, scale and octaves are 0..2.
            Self::FractalNoise { .. } | Self::Noise | Self::GradientNoise | Self::SimplexNoise => {
                &[
                    "uv",
                    "scale",
                    "octaves",
                    "roughness",
                    "lacunarity",
                    "distortion",
                    "phase",
                ]
            }
            Self::SceneColor => &["uv"],
            Self::WhiteNoise => &["uv", "scale"],
            Self::VoronoiNoise { .. } | Self::Voronoi => {
                &["uv", "scale", "randomness", "phase", "smoothness"]
            }
            Self::Circle => &["uv", "radius", "softness"],
            Self::Rectangle => &["uv", "size", "softness"],
            Self::Ring => &["uv", "radius", "thickness"],
            Self::Polygon => &["uv", "sides", "radius"],
            Self::Checker => &["uv", "tiles"],
            Self::ShaderOutput { kind } if kind == "unlit" || kind == "post_process" => {
                &["color", "alpha"]
            }
            Self::Output | Self::ShaderOutput { .. } => {
                &["base color", "normal", "metallic", "roughness", "emissive"]
            }
        }
    }

    /// A surface `ShaderOutput`: what the menu offers and a new graph starts from.
    pub(crate) fn surface_output() -> Self {
        Self::ShaderOutput {
            kind: "surface".to_owned(),
        }
    }

    /// The shader kind an output node writes; `None` for every other node.
    pub(crate) fn output_kind(&self) -> Option<&str> {
        match self {
            Self::Output => Some("surface"),
            Self::ShaderOutput { kind } => Some(kind),
            _ => None,
        }
    }

    /// Its output pins, named, and what each reads of the node's value. Pin 0 is the whole value
    /// wherever the whole means something, so a wire drawn before a node had channels still reads it;
    /// then one pin per channel, as a colour or a vector is taken apart in Unity and Unreal.
    pub(crate) fn outputs(&self) -> &'static [(&'static str, Pick)] {
        match self {
            Self::Output | Self::ShaderOutput { .. } => &[],
            Self::SceneColor
            | Self::Texture { .. }
            | Self::Color { .. }
            | Self::ConstColor(_)
            | Self::Blend { .. }
            | Self::Desaturate => RGBA,
            Self::WorldPosition | Self::WorldNormal | Self::ViewDirection | Self::UnpackNormal => {
                XYZ
            }
            Self::Uv
            | Self::Panner
            | Self::Rotator
            | Self::Tiling
            | Self::Twirl
            | Self::RadialShear
            | Self::Spherize => UV,
            Self::PolarCoordinates => POLAR,
            Self::Vector { width, .. } | Self::ConstVector { width, .. } => match width {
                2 => XY,
                3 => XYZ,
                _ => XYZW,
            },
            Self::Constant(_) => XYZW,
            // Packed into one wire these read as nonsense — a clock, its sine and its cosine are not
            // a colour — so they come apart only.
            Self::Time => TIME,
            Self::Voronoi | Self::VoronoiNoise { .. } => VORONOI,
            Self::FractalNoise { .. } | Self::Noise | Self::GradientNoise | Self::SimplexNoise => {
                FRACTAL
            }
            Self::Split => SPLIT,
            _ => WHOLE,
        }
    }

    /// What output pin `index` reads of the node's value, held in `value`.
    pub(crate) fn output_of(&self, value: &str, index: usize) -> String {
        match self.outputs().get(index).map(|&(_, pick)| pick) {
            None | Some(Pick::Whole) => value.to_owned(),
            Some(Pick::Rgb) => format!("vec4<f32>({value}.xyz, 0.0)"),
            Some(Pick::Yzw) => format!("vec4<f32>({value}.yzw, 0.0)"),
            Some(Pick::Field(field)) => format!("vec4<f32>({value}.{field})"),
            Some(Pick::FieldXy(field)) => format!("vec4<f32>({value}.{field}, 0.0, 0.0)"),
            Some(Pick::Channel(channel)) => {
                format!(
                    "vec4<f32>({value}.{})",
                    ["x", "y", "z", "w"][channel.min(3)]
                )
            }
        }
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
            | Self::SceneColor
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
            Self::Panner
            | Self::Rotator
            | Self::Tiling
            | Self::PolarCoordinates
            | Self::Twirl
            | Self::RadialShear
            | Self::Spherize => Category::Uv,
            Self::Fresnel | Self::UnpackNormal | Self::Desaturate | Self::Blend { .. } => {
                Category::Effect
            }
            Self::FractalNoise { .. }
            | Self::VoronoiNoise { .. }
            | Self::Noise
            | Self::GradientNoise
            | Self::SimplexNoise
            | Self::WhiteNoise
            | Self::Voronoi => Category::Noise,
            Self::Circle | Self::Rectangle | Self::Ring | Self::Polygon | Self::Checker => {
                Category::Shape
            }
            Self::Output | Self::ShaderOutput { .. } => Category::Output,
        }
    }

    /// Whether it is one of the noises, which all share `graph_hash`.
    pub(crate) fn is_noise(&self) -> bool {
        matches!(
            self,
            Self::FractalNoise { .. }
                | Self::VoronoiNoise { .. }
                | Self::Noise
                | Self::GradientNoise
                | Self::SimplexNoise
                | Self::WhiteNoise
                | Self::Voronoi
        )
    }
}
