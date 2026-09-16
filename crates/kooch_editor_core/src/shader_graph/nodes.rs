//! Every node the graph offers: what it is called, what it takes, and which menu it lives under
//! (#1159).
//!
//! 🔴 Every wire carries a `vec4<f32>`. Unused components are zero and each node reads the ones it
//! needs, so any output plugs into any input — no casts, no coercion rules, and no wire a user
//! cannot connect. A node that wants one number reads `.x`.

use serde::{Deserialize, Serialize};

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
    /// A value written into the graph rather than the material.
    Constant([f32; 4]),

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
    /// Value noise over a coordinate, 0..1.
    Noise,

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
    Math,
    Vector,
    Effect,
    Shape,
    Output,
}

impl Category {
    /// In menu order.
    pub(crate) const ALL: [Self; 6] = [
        Self::Input,
        Self::Math,
        Self::Vector,
        Self::Effect,
        Self::Shape,
        Self::Output,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Input => "Input",
            Self::Math => "Math",
            Self::Vector => "Vector",
            Self::Effect => "Effects",
            Self::Shape => "Shapes",
            Self::Output => "Output",
        }
    }
}

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
            Self::Fresnel => "Fresnel".to_owned(),
            Self::UnpackNormal => "Unpack Normal".to_owned(),
            Self::Panner => "Panner".to_owned(),
            Self::Rotator => "Rotator".to_owned(),
            Self::Tiling => "Tiling".to_owned(),
            Self::Desaturate => "Desaturate".to_owned(),
            Self::Blend { mode } => format!("Blend {mode}"),
            Self::Noise => "Noise".to_owned(),
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
            | Self::Constant(_) => &[],
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
            Self::Fresnel => &["power"],
            Self::UnpackNormal => &["map"],
            Self::Panner => &["uv", "speed"],
            Self::Rotator => &["uv", "centre", "turns"],
            Self::Tiling => &["uv", "tiling", "offset"],
            Self::Desaturate => &["colour", "amount"],
            Self::Blend { .. } => &["a", "b", "opacity"],
            Self::Noise => &["uv", "scale"],
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
        !matches!(self, Self::Output)
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
            | Self::Combine => Category::Vector,
            Self::Fresnel
            | Self::UnpackNormal
            | Self::Panner
            | Self::Rotator
            | Self::Tiling
            | Self::Desaturate
            | Self::Blend { .. }
            | Self::Noise => Category::Effect,
            Self::Circle | Self::Rectangle | Self::Ring | Self::Polygon | Self::Checker => {
                Category::Shape
            }
            Self::Output => Category::Output,
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
        Node::Constant([1.0; 4]),
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

/// What a parameter node declares as a member of `SurfaceParams`, whatever kind of node it is.
pub(crate) struct Declared<'a> {
    pub name: &'a str,
    pub width: u32,
    pub default: [f32; 4],
    /// What follows the member: `@color`, `@int`, `@range(lo, hi)`, or nothing.
    pub hint: String,
}

impl Node {
    /// The member this node declares, if it is a parameter.
    pub(crate) fn declared(&self) -> Option<Declared<'_>> {
        let range = |range: &Option<[f32; 2]>| {
            range.map_or(String::new(), |[lo, hi]| format!("@range({lo}, {hi})"))
        };
        let (name, width, default, hint) = match self {
            Self::Param {
                name,
                width,
                color,
                default,
            } => {
                // 🔴 Only at four wide: the engine refuses `@color` on anything narrower.
                let hint = if *color && *width == 4 { "@color" } else { "" };
                (name, (*width).clamp(1, 4), *default, hint.to_owned())
            }
            Self::Float {
                name,
                default,
                range: bounds,
            } => (name, 1, [*default, 0.0, 0.0, 0.0], range(bounds)),
            Self::Int {
                name,
                default,
                range: bounds,
            } => (
                name,
                1,
                [default.round(), 0.0, 0.0, 0.0],
                format!("@int {}", range(bounds)).trim_end().to_owned(),
            ),
            Self::Vector {
                name,
                width,
                default,
            } => (name, (*width).clamp(2, 4), *default, String::new()),
            Self::Color { name, default } => (name, 4, *default, "@color".to_owned()),
            _ => return None,
        };
        Some(Declared {
            name,
            width,
            default,
            hint,
        })
    }

    /// The typed node a pre-#1170 `Param` means, so an old graph opens as the new ones do.
    pub(crate) fn migrated(self) -> Self {
        let Self::Param {
            name,
            width,
            color,
            default,
        } = self
        else {
            return self;
        };
        match width {
            4 if color => Self::Color { name, default },
            0 | 1 => Self::Float {
                name,
                default: default[0],
                range: None,
            },
            width => Self::Vector {
                name,
                width: width.min(4),
                default,
            },
        }
    }
}
