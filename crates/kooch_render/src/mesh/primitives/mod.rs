//! Procedurally generated meshes — the shapes a scene needs before an
//! artist has made anything.
//!
//! Until now every mesh had to come from a `.glb`, so putting a floor
//! under a falling rigid body meant opening Blender. These are the same
//! six shapes every engine ships in its Add menu.
//!
//! # They are assets, not handles
//!
//! A primitive could be generated at startup and registered straight into
//! the meshlet pool, never touching the asset pipeline. They go through
//! it instead, as ordinary `.glb` files with committed GUIDs, because a
//! primitive that is a real asset can be inspected, replaced by a
//! hand-modelled version of the same name, and — the part that decides it
//! — the [exporter](crate::mesh::export) this needs is the same exporter
//! that turns a heavy mesh into a simplified collision mesh.
//!
//! # Conventions
//!
//! Right-handed, Y up, counter-clockwise front faces: glTF's rules, so a
//! generated mesh and an imported one behave identically. Round shapes
//! run along local Y, matching Rapier's `capsule_y` and what a character
//! controller assumes.

mod builder;
mod cone;
mod flat;
mod sphere;

use glam::{Vec2, Vec3};

use super::Mesh;

use builder::{MIN_RINGS, MIN_SECTORS};

/// Smallest dimension any primitive is built with.
///
/// Clamped rather than rejected: a value being typed into the Inspector
/// passes through zero on the way to the intended number, and a
/// zero-sized mesh produces NaN normals that outlive the typo.
pub const MIN_EXTENT: f32 = 1e-4;

/// A parametric shape, resolved to a [`Mesh`] by [`Primitive::build`].
///
/// Deliberately not a `Reflect` component: this is a *recipe*, evaluated
/// once at bake time into an asset. The scene references the resulting
/// asset by GUID like any other mesh, so nothing re-generates geometry at
/// load time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Primitive {
    /// Axis-aligned box centred on the origin.
    Cube { half_extents: Vec3 },
    /// Single-sided plane on XZ facing +Y — a floor.
    Quad { half_extents: Vec2 },
    /// UV sphere. `rings` are bands of latitude, `sectors` columns of
    /// longitude.
    Sphere {
        radius: f32,
        rings: u32,
        sectors: u32,
    },
    /// Capsule along local Y. Total height is `2 * (half_height + radius)`.
    Capsule {
        radius: f32,
        half_height: f32,
        rings: u32,
        sectors: u32,
    },
    /// Cylinder along local Y. Height is `2 * half_height`.
    Cylinder {
        radius: f32,
        half_height: f32,
        sectors: u32,
    },
    /// Cone along local Y, base at `-half_height`, apex at `+half_height`.
    Cone {
        radius: f32,
        half_height: f32,
        sectors: u32,
    },
}

impl Primitive {
    /// The canonical set baked into the engine's assets, in menu order.
    ///
    /// Unit-scaled so an artist can size them with the Transform gizmo
    /// rather than re-baking: a 1×1×1 cube, a sphere of diameter 1, and a
    /// capsule 2 units tall.
    pub const CANONICAL: [(&'static str, Primitive); 6] = [
        (
            "cube",
            Primitive::Cube {
                half_extents: Vec3::splat(0.5),
            },
        ),
        (
            "sphere",
            Primitive::Sphere {
                radius: 0.5,
                rings: 24,
                sectors: 32,
            },
        ),
        (
            "capsule",
            Primitive::Capsule {
                radius: 0.5,
                half_height: 0.5,
                rings: 16,
                sectors: 32,
            },
        ),
        (
            "cylinder",
            Primitive::Cylinder {
                radius: 0.5,
                half_height: 0.5,
                sectors: 32,
            },
        ),
        (
            "cone",
            Primitive::Cone {
                radius: 0.5,
                half_height: 0.5,
                sectors: 32,
            },
        ),
        (
            "plane",
            Primitive::Quad {
                half_extents: Vec2::splat(5.0),
            },
        ),
    ];

    /// Generates the geometry.
    pub fn build(&self) -> Mesh {
        match *self {
            Primitive::Cube { half_extents } => flat::cube(half_extents),
            Primitive::Quad { half_extents } => flat::quad(half_extents),
            Primitive::Sphere {
                radius,
                rings,
                sectors,
            } => sphere::sphere(radius, rings, sectors),
            Primitive::Capsule {
                radius,
                half_height,
                rings,
                sectors,
            } => sphere::capsule(radius, half_height, rings, sectors),
            Primitive::Cylinder {
                radius,
                half_height,
                sectors,
            } => cone::cylinder(radius, half_height, sectors),
            Primitive::Cone {
                radius,
                half_height,
                sectors,
            } => cone::cone(radius, half_height, sectors),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
