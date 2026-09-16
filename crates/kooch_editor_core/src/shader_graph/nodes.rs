//! Every node the graph offers: what it is called, what it takes, and which menu it lives under
//! (#1159).
//!
//! 🔴 Every wire carries a `vec4<f32>`. Unused components are zero and each node reads the ones it
//! needs, so any output plugs into any input — no casts, no coercion rules, and no wire a user
//! cannot connect. A node that wants one number reads `.x`.

use serde::{Deserialize, Serialize};

mod meta;
mod params;

/// One node of a graph.
///
/// 🔴 Serialised by name, so adding a variant keeps every graph already written readable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum Node {
    // -- Input ----------------------------------------------------------
    /// The mesh's uv, in `xy`.
    Uv,
    /// The shaded point, in world space.
    WorldPosition,
    /// The interpolated normal, in world space.
    WorldNormal,
    /// `camera_position - world_position`, normalised.
    ViewDirection,
    /// Seconds since the engine started: `x` raw, `y` its sine, `z` its cosine, `w` a tenth of it.
    Time,
    /// A number the material edits, from before parameters were typed (#1170). Still read so graphs
    /// written then open; `migrated` turns it into one of the typed nodes below, and the menu never
    /// offers it.
    Param {
        name: String,
        /// How wide the member is, 1..=4.
        width: u32,
        /// Drawn as a colour. Only at four wide — the hint the engine reads is a `vec4<f32>` one.
        color: bool,
        default: [f32; 4],
    },
    /// A float the material edits — a slider when it has a range.
    Float {
        name: String,
        default: f32,
        range: Option<[f32; 2]>,
    },
    /// A whole number the material edits: an `f32` hinted `@int`, stepped by one.
    Int {
        name: String,
        default: f32,
        range: Option<[f32; 2]>,
    },
    /// A vector the material edits, two to four wide.
    Vector {
        name: String,
        width: u32,
        default: [f32; 4],
    },
    /// A colour the material edits with a picker.
    Color {
        name: String,
        default: [f32; 4],
    },
    /// A texture the material assigns, sampled at `uv`.
    Texture {
        name: String,
        /// `white`, `black` or `normal` while unassigned.
        fallback: String,
        /// An image to see the preview with. Rides in the graph like a node's position and reaches
        /// neither the WGSL nor a material: which image a material samples is the material's call.
        #[serde(default)]
        preview: Option<kooch_core::Guid>,
    },
    /// A value written into the graph, from before constants were typed. Read so old graphs open;
    /// `migrated` turns it into a four-wide vector, and the menu never offers it.
    Constant([f32; 4]),
    /// A number written into the shader rather than the material.
    ConstFloat(f32),
    /// A whole number written into the shader.
    ConstInt(f32),
    /// A vector written into the shader, two to four wide.
    ConstVector {
        width: u32,
        value: [f32; 4],
    },
    /// A colour written into the shader.
    ConstColor([f32; 4]),

    // -- Math -----------------------------------------------------------
    Add,
    Subtract,
    Multiply,
    Divide,
    /// `1 - value`.
    OneMinus,
    Abs,
    Floor,
    /// The fractional part — what tiles a coordinate.
    Fract,
    Sine,
    Cosine,
    Min,
    Max,
    /// `clamp(value, low.x, high.x)`.
    Clamp,
    /// 0 below the edge, 1 above it.
    Step,
    /// A smooth ramp between two edges.
    Smoothstep,
    /// `pow(max(a.x, 0), b.x)`.
    Power,
    /// Clamped to 0..1.
    Saturate,
    /// From one range to another: `from` is `xy`, `to` is `xy`.
    Remap,
    /// `mix(a, b, t.x)`.
    Mix,

    // -- Vector ---------------------------------------------------------
    /// `dot(a.xyz, b.xyz)`, in every component.
    Dot,
    Cross,
    Normalize,
    /// The length of `xyz`, in every component.
    Length,
    Distance,
    /// `reflect(incident.xyz, normal.xyz)`.
    Reflect,
    /// Reorders components: `xyzw` passes through, `xxxx` splashes the first, `yx` is a swap.
    Swizzle {
        pattern: String,
    },
    /// Four numbers into one vector, each read from its input's `x`.
    Combine,
    /// One vector into its four components, one output each — the other half of `Combine`.
    Split,

    // -- Effects --------------------------------------------------------
    /// Bright at grazing angles: the rim of a sphere. `pow(1 - dot(N, V), power)`.
    Fresnel,
    /// A tangent-space normal map into world space, through the mesh's tangent frame.
    UnpackNormal,
    /// Scrolls a coordinate over time: `uv + speed * time`.
    Panner,
    /// Turns a coordinate around a centre by an angle in turns.
    Rotator,
    /// `uv * tiling + offset`.
    Tiling,
    /// Towards grey, by amount.
    Desaturate,
    /// Two colours combined the way an image editor does.
    Blend {
        /// `multiply`, `screen`, `overlay`, `lighten` or `darken`.
        mode: String,
    },
    // -- Noise ----------------------------------------------------------
    /// Value noise over a coordinate, 0..1. Named `Noise` because it was the first; the menu calls
    /// it Value Noise.
    Noise,
    /// Gradient (Perlin) noise, 0..1.
    GradientNoise,
    /// Simplex noise, 0..1.
    SimplexNoise,
    /// One random value per cell.
    WhiteNoise,
    /// Cellular noise: F1, F2, F2 - F1 and a random value per cell, in x, y, z and w.
    Voronoi,

    // -- Shapes ---------------------------------------------------------
    /// A disc around the middle of the uv square.
    Circle,
    /// A box around the middle of the uv square.
    Rectangle,
    /// A circle with its middle cut out.
    Ring,
    /// A regular polygon, by side count.
    Polygon,
    /// A checkerboard.
    Checker,

    // -- Output ---------------------------------------------------------
    /// What Inti lights: base colour, normal, metallic, roughness, emissive.
    Output,
}

/// Which submenu a node is added from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Category {
    Input,
    /// Values written into the shader: the same types as the parameters, fixed at authoring.
    Constant,
    Math,
    Vector,
    Effect,
    Shape,
    Noise,
    Output,
}

impl Category {
    /// In menu order.
    pub(crate) const ALL: [Self; 8] = [
        Self::Input,
        Self::Constant,
        Self::Math,
        Self::Vector,
        Self::Effect,
        Self::Shape,
        Self::Noise,
        Self::Output,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Input => "Input",
            Self::Constant => "Constants",
            Self::Math => "Math",
            Self::Vector => "Vector",
            Self::Effect => "Effects",
            Self::Shape => "Shapes",
            Self::Noise => "Noise",
            Self::Output => "Output",
        }
    }
}

/// Every node the menu offers, one per kind, in menu order.
pub(crate) fn palette() -> Vec<Node> {
    vec![
        Node::Uv,
        Node::WorldPosition,
        Node::WorldNormal,
        Node::ViewDirection,
        Node::Time,
        Node::Float {
            name: "amount".to_owned(),
            default: 0.5,
            range: Some([0.0, 1.0]),
        },
        Node::Int {
            name: "count".to_owned(),
            default: 1.0,
            range: None,
        },
        Node::Vector {
            name: "offset".to_owned(),
            width: 2,
            default: [0.0; 4],
        },
        Node::Vector {
            name: "direction".to_owned(),
            width: 3,
            default: [0.0; 4],
        },
        Node::Vector {
            name: "vector".to_owned(),
            width: 4,
            default: [0.0; 4],
        },
        Node::Color {
            name: "tint".to_owned(),
            default: [1.0; 4],
        },
        Node::Texture {
            name: "map".to_owned(),
            fallback: "white".to_owned(),
            preview: None,
        },
        Node::ConstFloat(1.0),
        Node::ConstInt(1.0),
        Node::ConstVector {
            width: 2,
            value: [0.0; 4],
        },
        Node::ConstVector {
            width: 3,
            value: [0.0; 4],
        },
        Node::ConstVector {
            width: 4,
            value: [0.0; 4],
        },
        Node::ConstColor([1.0; 4]),
        Node::Add,
        Node::Subtract,
        Node::Multiply,
        Node::Divide,
        Node::OneMinus,
        Node::Abs,
        Node::Floor,
        Node::Fract,
        Node::Sine,
        Node::Cosine,
        Node::Min,
        Node::Max,
        Node::Clamp,
        Node::Step,
        Node::Smoothstep,
        Node::Power,
        Node::Saturate,
        Node::Remap,
        Node::Mix,
        Node::Dot,
        Node::Cross,
        Node::Normalize,
        Node::Length,
        Node::Distance,
        Node::Reflect,
        Node::Swizzle {
            pattern: "xyzw".to_owned(),
        },
        Node::Combine,
        Node::Split,
        Node::Fresnel,
        Node::UnpackNormal,
        Node::Panner,
        Node::Rotator,
        Node::Tiling,
        Node::Desaturate,
        Node::Blend {
            mode: "multiply".to_owned(),
        },
        Node::Noise,
        Node::GradientNoise,
        Node::SimplexNoise,
        Node::WhiteNoise,
        Node::Voronoi,
        Node::Circle,
        Node::Rectangle,
        Node::Ring,
        Node::Polygon,
        Node::Checker,
        Node::Output,
    ]
}

/// What a `Blend` node offers, and what the codegen knows how to write.
pub(crate) const BLEND_MODES: [&str; 5] = ["multiply", "screen", "overlay", "lighten", "darken"];

/// What a `Texture` node falls back to while nothing is assigned.
pub(crate) const TEXTURE_FALLBACKS: [&str; 3] = ["white", "black", "normal"];
