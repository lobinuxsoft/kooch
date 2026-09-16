//! Parameter nodes as the `SurfaceParams` members they declare, and the typed nodes an old graph's
//! nodes mean (#1170).

use super::Node;

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
        match self {
            Self::Param {
                name,
                color: true,
                width: 4,
                default,
            } => Self::Color { name, default },
            Self::Param {
                name,
                width: 0 | 1,
                default,
                ..
            } => Self::Float {
                name,
                default: default[0],
                range: None,
            },
            Self::Param {
                name,
                width,
                default,
                ..
            } => Self::Vector {
                name,
                width: width.min(4),
                default,
            },
            // The noises before their controls existed: same pins in the same places, fBm.
            Self::Noise | Self::GradientNoise | Self::SimplexNoise => {
                let basis = match self {
                    Self::GradientNoise => "gradient",
                    Self::SimplexNoise => "simplex",
                    _ => "value",
                };
                Self::FractalNoise {
                    basis: basis.to_owned(),
                    fractal: "fbm".to_owned(),
                }
            }
            Self::Voronoi => Self::VoronoiNoise {
                metric: "euclidean".to_owned(),
            },
            // All four components were always live, so the vector keeps all four.
            Self::Constant(value) => Self::ConstVector { width: 4, value },
            node => node,
        }
    }
}
