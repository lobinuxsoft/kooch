//! What each node does and what each of its pins takes and gives, for pin labels, colours and
//! tooltips (#1159). Every wire is still a `vec4<f32>`: a width says how much of it a pin reads.

use super::Node;
use super::meta::Pick;

/// How many components a pin reads or gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Width {
    One,
    Two,
    Three,
    Four,
    /// Component by component, whatever comes in.
    Any,
}

impl Width {
    /// Unity's `(2)` after a pin name; nothing for a pin that takes anything.
    pub(crate) fn suffix(self) -> &'static str {
        match self {
            Self::One => " (1)",
            Self::Two => " (2)",
            Self::Three => " (3)",
            Self::Four => " (4)",
            Self::Any => "",
        }
    }

    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::One => "a number (x)",
            Self::Two => "a 2D vector (xy)",
            Self::Three => "a 3D vector or colour (xyz)",
            Self::Four => "a 4D vector or colour with alpha (xyzw)",
            Self::Any => "any value, component by component",
        }
    }
}

use Width::{Any, Four, One, Three, Two};

const UV_IN: (Width, &str) = (Two, "The coordinate to read. Unwired: the mesh's uv.");
const UV_ZERO: (Width, &str) = (Two, "The coordinate to read. Unwired: zero — wire a UV in.");
const CENTRE: (Width, &str) = (Two, "The point it works around. Unwired: the middle, 0.5.");
const OFFSET: (Width, &str) = (Two, "Added to the result, to move it.");

impl Node {
    /// One line on what the node does.
    pub(crate) fn about(&self) -> &'static str {
        match self {
            Self::Uv => "The mesh's texture coordinate.",
            Self::SceneColor => "The frame the camera produced. Post-process shaders only.",
            Self::WorldPosition => "The shaded point, in world space.",
            Self::WorldNormal => "The surface's normal, in world space.",
            Self::ViewDirection => "From the surface towards the camera, normalised.",
            Self::Time => "The clock, in seconds, and waves made from it.",
            Self::Float { .. } | Self::Param { .. } => "A number the material edits.",
            Self::Int { .. } => "A whole number the material edits.",
            Self::Vector { .. } => "A vector the material edits.",
            Self::Color { .. } => "A colour the material edits.",
            Self::Texture { .. } => "A texture the material assigns, sampled at a coordinate.",
            Self::ConstFloat(_) => "A fixed number.",
            Self::ConstInt(_) => "A fixed whole number.",
            Self::ConstVector { .. } | Self::Constant(_) => "A fixed vector.",
            Self::ConstColor(_) => "A fixed colour.",
            Self::Add => "a + b.",
            Self::Subtract => "a - b.",
            Self::Multiply => "a × b: scales, tints and masks.",
            Self::Divide => "a ÷ b, safe at zero.",
            Self::OneMinus => "1 - value: inverts a mask.",
            Self::Abs => "The value without its sign.",
            Self::Floor => "Rounds down: steps.",
            Self::Fract => "The part after the point: repeats a coordinate.",
            Self::Sine => "A wave from -1 to 1.",
            Self::Cosine => "A wave from -1 to 1, a quarter turn after the sine.",
            Self::Min => "The smaller of a and b.",
            Self::Max => "The larger of a and b.",
            Self::Clamp => "Holds a value between low and high.",
            Self::Step => "0 below the edge, 1 from it on.",
            Self::Smoothstep => "A smooth ramp from 0 at low to 1 at high.",
            Self::Power => "a raised to b: sharpens or softens a 0..1 ramp.",
            Self::Saturate => "Holds a value between 0 and 1.",
            Self::Remap => "Moves a value from one range to another.",
            Self::Mix => "Blends from a to b as t goes from 0 to 1.",
            Self::Dot => "How much a and b point the same way.",
            Self::Cross => "The direction square to both a and b.",
            Self::Normalize => "The same direction, length 1.",
            Self::Length => "How long a vector is.",
            Self::Distance => "How far apart two points are.",
            Self::Reflect => "Bounces a direction off a surface.",
            Self::Swizzle { .. } => "Picks and reorders components: yx swaps, xxx repeats.",
            Self::Combine => "Four numbers into one vector.",
            Self::Split => "A vector into its components.",
            Self::Fresnel => "Bright where the surface turns away from the camera: rims.",
            Self::UnpackNormal => "A normal map's colour into a world-space normal.",
            Self::Panner => "Scrolls a coordinate over time.",
            Self::Rotator => "Turns a coordinate around a centre.",
            Self::Tiling => "Repeats and moves a coordinate.",
            Self::PolarCoordinates => "A coordinate as distance and angle from a centre.",
            Self::Twirl => "Swirls a coordinate around a centre.",
            Self::RadialShear => "Shears a coordinate into a spiral.",
            Self::Spherize => "Bulges a coordinate out like a lens.",
            Self::Desaturate => "Takes a colour towards grey.",
            Self::Blend { .. } => "Combines two colours the way an image editor does.",
            Self::FractalNoise { .. } | Self::Noise | Self::GradientNoise | Self::SimplexNoise => {
                "Smooth random noise, layered for detail."
            }
            Self::WhiteNoise => "One random value per cell.",
            Self::VoronoiNoise { .. } | Self::Voronoi => "Cells around random points.",
            Self::Circle => "A disc in the middle of the uv square.",
            Self::Rectangle => "A box in the middle of the uv square.",
            Self::Ring => "A circle with its middle cut out.",
            Self::Polygon => "A regular polygon.",
            Self::Checker => "A checkerboard.",
            Self::Output | Self::ShaderOutput { .. } => {
                "What the shader ends up as. Its kind picks what it takes."
            }
        }
    }

    /// Each input's width and what it is for, in pin order.
    pub(crate) fn input_docs(&self) -> &'static [(Width, &'static str)] {
        match self {
            Self::Texture { .. } => &[UV_ZERO],
            Self::Add | Self::Subtract | Self::Multiply | Self::Divide | Self::Min | Self::Max => {
                &[(Any, "The first value."), (Any, "The second value.")]
            }
            Self::OneMinus
            | Self::Abs
            | Self::Floor
            | Self::Fract
            | Self::Sine
            | Self::Cosine
            | Self::Saturate
            | Self::Split
            | Self::Swizzle { .. } => &[(Any, "The value.")],
            Self::Normalize | Self::Length => &[(Three, "The vector.")],
            Self::Power => &[(One, "The base, from 0 up."), (One, "The exponent.")],
            Self::Dot | Self::Cross => {
                &[(Three, "The first vector."), (Three, "The second vector.")]
            }
            Self::Distance => &[(Three, "The first point."), (Three, "The second point.")],
            Self::Clamp => &[
                (Any, "The value to hold."),
                (One, "The lowest it may be."),
                (One, "The highest it may be."),
            ],
            Self::Step => &[(One, "Where 0 turns into 1."), (Any, "The value compared.")],
            Self::Smoothstep => &[
                (One, "Where the ramp starts, at 0."),
                (One, "Where the ramp ends, at 1."),
                (Any, "The value on the ramp."),
            ],
            Self::Remap => &[
                (Any, "The value to move."),
                (Two, "The range it is in: x low, y high."),
                (Two, "The range it goes to: x low, y high."),
            ],
            Self::Mix => &[
                (Any, "What t 0 gives."),
                (Any, "What t 1 gives."),
                (One, "How far from a to b, 0..1."),
            ],
            Self::Reflect => &[
                (Three, "The direction coming in."),
                (Three, "The surface's normal."),
            ],
            Self::Combine => &[
                (One, "Becomes x."),
                (One, "Becomes y."),
                (One, "Becomes z."),
                (One, "Becomes w."),
            ],
            Self::Fresnel => &[(One, "How thin the rim is: higher, thinner.")],
            Self::UnpackNormal => &[(Three, "The normal map's colour.")],
            Self::Panner => &[
                UV_ZERO,
                (
                    Two,
                    "How far it moves per second: x along U, y along V. Time is already applied — \
                     wire a speed, not speed × Time.",
                ),
            ],
            Self::Rotator => &[
                UV_ZERO,
                (Two, "The point it turns around. Unwired: zero, the corner."),
                (One, "How far it turns, in whole turns."),
            ],
            Self::Tiling => &[
                UV_ZERO,
                (Two, "How many times it repeats: x along U, y along V."),
                OFFSET,
            ],
            Self::PolarCoordinates => &[
                UV_IN,
                CENTRE,
                (One, "Multiplies the radius. Unwired: 1."),
                (
                    One,
                    "Multiplies the angle: how many times it wraps. Unwired: 1.",
                ),
            ],
            Self::Twirl => &[
                UV_IN,
                CENTRE,
                (One, "How much it swirls. Unwired: 10."),
                OFFSET,
            ],
            Self::RadialShear | Self::Spherize => &[
                UV_IN,
                CENTRE,
                (One, "How strong it is. Unwired: 10."),
                OFFSET,
            ],
            Self::Desaturate => &[(Four, "The colour."), (One, "How grey, 0..1.")],
            Self::Blend { .. } => &[
                (Four, "The colour underneath."),
                (Four, "The colour on top."),
                (One, "How much of the blend shows, 0..1."),
            ],
            Self::FractalNoise { .. } | Self::Noise | Self::GradientNoise | Self::SimplexNoise => {
                &[
                    UV_ZERO,
                    (One, "How many cells per unit: higher, finer."),
                    (One, "Layers of detail, 1..8. Unwired: 1."),
                    (
                        One,
                        "How much each layer keeps of the last, 0..1. Unwired: 0.5.",
                    ),
                    (One, "How much finer each layer is. Unwired: 2."),
                    (One, "Warps the noise by itself: marble, smoke."),
                    (
                        One,
                        "Changes the noise in place: wire Time in to animate it.",
                    ),
                ]
            }
            Self::WhiteNoise => &[UV_ZERO, (One, "How many cells per unit.")],
            Self::VoronoiNoise { .. } | Self::Voronoi => &[
                UV_ZERO,
                (One, "How many cells per unit."),
                (One, "0 a regular grid, 1 fully random. Unwired: 1."),
                (One, "Moves the points in circles: wire Time in to animate."),
                (One, "Rounds F1, F2 and the border, 0..1."),
            ],
            Self::Circle => &[
                UV_ZERO,
                (One, "The radius, in uv units."),
                (One, "How soft its edge is."),
            ],
            Self::Rectangle => &[
                UV_ZERO,
                (Two, "Width and height, in uv units."),
                (One, "How soft its edge is."),
            ],
            Self::Ring => &[
                UV_ZERO,
                (One, "The radius, in uv units."),
                (One, "How thick the ring is."),
            ],
            Self::Polygon => &[
                UV_ZERO,
                (One, "How many sides, from 3."),
                (One, "The radius, in uv units."),
            ],
            Self::Checker => &[UV_ZERO, (Two, "How many squares: x across, y down.")],
            Self::SceneColor => &[(Two, "Where to read the frame. Unwired: this pixel.")],
            Self::ShaderOutput { kind } if kind == "unlit" => &[
                (Three, "The final colour. No light or shadow changes it."),
                (One, "Opacity, tested against alpha clip. Unwired: 1."),
                (One, "Cuts the surface where alpha falls below it. Unwired: nothing is cut."),
            ],
            Self::ShaderOutput { kind } if kind == "transparent" => &[
                (Three, "The colour of the surface."),
                (Three, "The world-space normal. Unwired: the mesh's."),
                (One, "0 dielectric, 1 metal."),
                (One, "0 mirror, 1 matte. Unwired: 0.5."),
                (Three, "Light the surface gives off."),
                (
                    One,
                    "How much it covers what is behind: 0 invisible, 1 solid. Unwired: 1.",
                ),
            ],
            Self::Output | Self::ShaderOutput { .. } => &[
                (Three, "The colour of the surface."),
                (Three, "The world-space normal. Unwired: the mesh's."),
                (One, "0 dielectric, 1 metal."),
                (One, "0 mirror, 1 matte. Unwired: 0.5."),
                (Three, "Light the surface gives off."),
                (One, "Opacity, tested against alpha clip. Unwired: 1."),
                (One, "Cuts the surface where alpha falls below it. Unwired: nothing is cut."),
            ],
            _ => &[],
        }
    }

    /// The width of output `index` and what it gives.
    pub(crate) fn output_doc(&self, index: usize) -> (Width, &'static str) {
        let (whole, about) = self.whole_doc();
        let Some(&(_, pick)) = self.outputs().get(index) else {
            return (whole, about);
        };
        match (self, pick) {
            (Self::Time, Pick::Channel(0)) => (One, "Seconds since the engine started."),
            (Self::Time, Pick::Channel(1)) => (One, "The sine of the time: -1..1, once per 2π s."),
            (Self::Time, Pick::Channel(2)) => (One, "The cosine of the time."),
            (Self::Time, _) => (One, "A tenth of the time: slow motion."),
            (_, Pick::Field("f1")) => (One, "Distance to the nearest point."),
            (_, Pick::Field("f2")) => (One, "Distance to the second nearest point."),
            (_, Pick::Field("edge")) => (One, "Distance to the nearest cell border."),
            (_, Pick::Field(_)) => (One, "A random value per cell, 0..1."),
            (_, Pick::FieldXy(_)) => (Two, "Where the nearest point is."),
            (Self::PolarCoordinates, Pick::Channel(0)) => (One, "Distance from the centre."),
            (Self::PolarCoordinates, Pick::Channel(1)) => (One, "Angle around the centre, 0..1."),
            (_, Pick::Yzw) => (Three, "A colour: three unrelated samples of the noise."),
            (_, Pick::Rgb) => (Three, "The colour without alpha."),
            (_, Pick::Channel(_)) => (One, "One component of the whole value."),
            (_, Pick::Whole) => (whole, about),
        }
    }

    fn whole_doc(&self) -> (Width, &'static str) {
        match self {
            Self::Uv => (Two, "The uv."),
            Self::WorldPosition | Self::WorldNormal | Self::ViewDirection => (Three, self.about()),
            Self::Float { .. } | Self::Int { .. } | Self::ConstFloat(_) | Self::ConstInt(_) => {
                (One, "The number.")
            }
            Self::Vector { width, .. } | Self::ConstVector { width, .. } => (
                match width {
                    2 => Two,
                    3 => Three,
                    _ => Four,
                },
                "The vector.",
            ),
            Self::Param { .. } | Self::Constant(_) | Self::Combine => (Four, "The vector."),
            Self::Color { .. } | Self::ConstColor(_) => (Four, "The colour, with alpha."),
            Self::Texture { .. } => (Four, "The sampled colour, with alpha."),
            Self::Desaturate | Self::Blend { .. } => (Four, "The resulting colour."),
            Self::Dot | Self::Length | Self::Distance | Self::Fresnel => (One, "The result."),
            Self::Cross | Self::Normalize | Self::Reflect | Self::UnpackNormal => {
                (Three, "The resulting direction.")
            }
            Self::Panner
            | Self::Rotator
            | Self::Tiling
            | Self::Twirl
            | Self::RadialShear
            | Self::Spherize => (Two, "The new coordinate."),
            Self::PolarCoordinates => (Two, "x the distance, y the angle."),
            Self::FractalNoise { .. }
            | Self::Noise
            | Self::GradientNoise
            | Self::SimplexNoise
            | Self::WhiteNoise => (One, "The noise, 0..1."),
            Self::Circle | Self::Rectangle | Self::Ring | Self::Polygon | Self::Checker => {
                (One, "A mask: 1 inside, 0 outside.")
            }
            _ => (Any, "The result, component by component."),
        }
    }
}
