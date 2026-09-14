//! [`BlockShape`] — the parameters a block was generated from, kept on the entity so the Inspector
//! can reshape it until its geometry is edited by hand (#1106).

use glam::Vec3;
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;
use kooch_ecs::reflect::{FieldChoice, FieldCondition};

use crate::Shape;

/// A box. Scenes store the kind as a number, so kinds only append.
pub const KIND_CUBE: u32 = 0;
/// Steps rising along +Z.
pub const KIND_STAIRS: u32 = 1;
/// One slope over the stairs' footprint.
pub const KIND_RAMP: u32 = 2;
/// A half ring on its feet.
pub const KIND_ARCH: u32 = 3;
/// A prism.
pub const KIND_CYLINDER: u32 = 4;
/// A prism collapsing to a point.
pub const KIND_CONE: u32 = 5;
/// A floor slab.
pub const KIND_PLANE: u32 = 6;

/// Labels for the `kind` dropdown, in the order of [`Shape::DEFAULTS`].
pub static KIND_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "Cube",
        value: KIND_CUBE as i64,
    },
    FieldChoice {
        label: "Stairs",
        value: KIND_STAIRS as i64,
    },
    FieldChoice {
        label: "Ramp",
        value: KIND_RAMP as i64,
    },
    FieldChoice {
        label: "Arch",
        value: KIND_ARCH as i64,
    },
    FieldChoice {
        label: "Cylinder",
        value: KIND_CYLINDER as i64,
    },
    FieldChoice {
        label: "Cone",
        value: KIND_CONE as i64,
    },
    FieldChoice {
        label: "Plane",
        value: KIND_PLANE as i64,
    },
];

/// Kinds that read `size`.
pub static SIZE_WHEN: FieldCondition = FieldCondition {
    field: "kind",
    values: &[KIND_CUBE as i64],
};
/// Kinds that read `steps`.
pub static STEPS_WHEN: FieldCondition = FieldCondition {
    field: "kind",
    values: &[KIND_STAIRS as i64],
};
/// Kinds that read `width`, `rise` and `run`.
pub static SLOPE_WHEN: FieldCondition = FieldCondition {
    field: "kind",
    values: &[KIND_STAIRS as i64, KIND_RAMP as i64],
};
/// Kinds that read the arch's fields.
pub static ARCH_WHEN: FieldCondition = FieldCondition {
    field: "kind",
    values: &[KIND_ARCH as i64],
};
/// Kinds that read `sides`, `radius` and `height`.
pub static ROUND_WHEN: FieldCondition = FieldCondition {
    field: "kind",
    values: &[KIND_CYLINDER as i64, KIND_CONE as i64],
};
/// Kinds that read the plane's fields.
pub static PLANE_WHEN: FieldCondition = FieldCondition {
    field: "kind",
    values: &[KIND_PLANE as i64],
};

/// The shape a block regenerates from whenever these change. Every kind's fields sit side by side,
/// because reflection has no enums; hidden ones keep their values.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Level")]
pub struct BlockShape {
    /// Which shape to generate. One of the `KIND_*` constants.
    #[reflect(choices = KIND_CHOICES)]
    pub kind: u32,
    /// Where the entity's origin sits in the shape's bounding box, each axis from -1 to 1:
    /// `(0, -1, 0)` is the centre of the base, `(-1, -1, -1)` a lower corner.
    pub pivot: Vec3,
    /// Box size along each axis.
    #[reflect(shown_when = SIZE_WHEN)]
    pub size: Vec3,
    /// How many steps the stairs have.
    #[reflect(shown_when = STEPS_WHEN)]
    pub steps: u32,
    /// Width across the slope.
    #[reflect(shown_when = SLOPE_WHEN)]
    pub width: f32,
    /// Total height of the slope.
    #[reflect(shown_when = SLOPE_WHEN)]
    pub rise: f32,
    /// Total depth of the slope.
    #[reflect(shown_when = SLOPE_WHEN)]
    pub run: f32,
    /// Segments around the arch.
    #[reflect(shown_when = ARCH_WHEN)]
    pub segments: u32,
    /// Radius of the arch's opening.
    #[reflect(shown_when = ARCH_WHEN)]
    pub opening: f32,
    /// Thickness of the arch's wall.
    #[reflect(shown_when = ARCH_WHEN)]
    pub wall: f32,
    /// Depth of the arch.
    #[reflect(shown_when = ARCH_WHEN)]
    pub depth: f32,
    /// Faces around a cylinder or cone.
    #[reflect(shown_when = ROUND_WHEN)]
    pub sides: u32,
    /// Radius of a cylinder or cone.
    #[reflect(shown_when = ROUND_WHEN)]
    pub radius: f32,
    /// Height of a cylinder or cone.
    #[reflect(shown_when = ROUND_WHEN)]
    pub height: f32,
    /// Quads along each side of the plane.
    #[reflect(shown_when = PLANE_WHEN)]
    pub subdivisions: u32,
    /// Side length of the plane.
    #[reflect(shown_when = PLANE_WHEN)]
    pub extent: f32,
    /// Thickness of the plane.
    #[reflect(shown_when = PLANE_WHEN)]
    pub thickness: f32,
}

impl Component for BlockShape {}

impl Default for BlockShape {
    /// 🔴 Must match [`Shape::DEFAULTS`]: a remote spawn writes only `kind` and relies on the rest.
    fn default() -> Self {
        Self {
            kind: KIND_CUBE,
            // The base, so a spawned block stands on the grid rather than sinking half into it.
            pivot: Vec3::new(0.0, -1.0, 0.0),
            size: Vec3::ONE,
            steps: 4,
            width: 1.0,
            rise: 1.0,
            run: 2.0,
            segments: 8,
            opening: 1.0,
            wall: 0.25,
            depth: 0.5,
            sides: 16,
            radius: 0.5,
            height: 1.0,
            subdivisions: 4,
            extent: 4.0,
            thickness: 0.1,
        }
    }
}

impl From<Shape> for BlockShape {
    fn from(shape: Shape) -> Self {
        let mut out = Self::default();
        match shape {
            Shape::Cube { size } => {
                out.kind = KIND_CUBE;
                out.size = size;
            }
            Shape::Stairs {
                steps,
                width,
                rise,
                run,
            } => {
                out.kind = KIND_STAIRS;
                (out.steps, out.width, out.rise, out.run) = (steps, width, rise, run);
            }
            Shape::Ramp { width, rise, run } => {
                out.kind = KIND_RAMP;
                (out.width, out.rise, out.run) = (width, rise, run);
            }
            Shape::Arch {
                segments,
                radius,
                thickness,
                depth,
            } => {
                out.kind = KIND_ARCH;
                (out.segments, out.opening, out.wall, out.depth) =
                    (segments, radius, thickness, depth);
            }
            Shape::Cylinder {
                sides,
                radius,
                height,
            } => {
                out.kind = KIND_CYLINDER;
                (out.sides, out.radius, out.height) = (sides, radius, height);
            }
            Shape::Cone {
                sides,
                radius,
                height,
            } => {
                out.kind = KIND_CONE;
                (out.sides, out.radius, out.height) = (sides, radius, height);
            }
            Shape::Plane {
                subdivisions,
                size,
                thickness,
            } => {
                out.kind = KIND_PLANE;
                (out.subdivisions, out.extent, out.thickness) = (subdivisions, size, thickness);
            }
        }
        out
    }
}

impl BlockShape {
    /// The mesh these fields describe, moved so `pivot` lands on the origin.
    pub fn build(&self) -> crate::BlockMesh {
        let mut mesh = self.shape().build();
        let Some(first) = mesh.positions.first().copied() else {
            return mesh;
        };
        let (min, max) = mesh
            .positions
            .iter()
            .fold((first, first), |(min, max), p| (min.min(*p), max.max(*p)));
        let centre = (min + max) / 2.0;
        let point = centre + (max - min) / 2.0 * self.pivot.clamp(Vec3::NEG_ONE, Vec3::ONE);
        for position in &mut mesh.positions {
            *position -= point;
        }
        mesh
    }

    /// The shape these fields describe; an unknown kind falls back to a cube so a newer scene
    /// still loads.
    pub fn shape(&self) -> Shape {
        match self.kind {
            KIND_STAIRS => Shape::Stairs {
                steps: self.steps,
                width: self.width,
                rise: self.rise,
                run: self.run,
            },
            KIND_RAMP => Shape::Ramp {
                width: self.width,
                rise: self.rise,
                run: self.run,
            },
            KIND_ARCH => Shape::Arch {
                segments: self.segments,
                radius: self.opening,
                thickness: self.wall,
                depth: self.depth,
            },
            KIND_CYLINDER => Shape::Cylinder {
                sides: self.sides,
                radius: self.radius,
                height: self.height,
            },
            KIND_CONE => Shape::Cone {
                sides: self.sides,
                radius: self.radius,
                height: self.height,
            },
            KIND_PLANE => Shape::Plane {
                subdivisions: self.subdivisions,
                size: self.extent,
                thickness: self.thickness,
            },
            _ => Shape::Cube { size: self.size },
        }
    }
}

#[cfg(test)]
mod tests;
