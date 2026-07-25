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
pub(crate) mod tests {
    use super::*;

    /// Every normal is unit length. A zero or over-long normal darkens or
    /// blows out the shading, and looks like a lighting bug.
    pub(crate) fn assert_unit_normals(mesh: &Mesh) {
        for (i, v) in mesh.vertices.iter().enumerate() {
            let len = Vec3::from_array(v.normal).length();
            assert!(
                (len - 1.0).abs() < 1e-3,
                "vertex {i} normal is not unit length: {len}"
            );
        }
    }

    /// UVs stay inside `[0, 1]`. Outside it a clamped sampler smears the
    /// edge texel across the whole face.
    pub(crate) fn assert_uvs_in_unit_range(mesh: &Mesh) {
        for (i, v) in mesh.vertices.iter().enumerate() {
            assert!(
                (-1e-4..=1.0 + 1e-4).contains(&v.uv[0]) && (-1e-4..=1.0 + 1e-4).contains(&v.uv[1]),
                "vertex {i} uv out of range: {:?}",
                v.uv
            );
        }
    }

    /// Winding agrees with the vertex normals: the geometric normal of
    /// every non-degenerate triangle points the same way as its corners'.
    ///
    /// This is the assertion that catches an inside-out primitive, which
    /// otherwise only shows up as an invisible mesh once backface culling
    /// is on.
    pub(crate) fn assert_outward_facing(mesh: &Mesh) {
        let position = |i: u32| Vec3::from_array(mesh.vertices[i as usize].position);
        let normal = |i: u32| Vec3::from_array(mesh.vertices[i as usize].normal);
        for (t, tri) in mesh.indices.chunks(3).enumerate() {
            let (a, b, c) = (tri[0], tri[1], tri[2]);
            let geometric = (position(b) - position(a)).cross(position(c) - position(a));
            // Degenerate triangles (the poles of a UV sphere) have no
            // geometric normal to compare against.
            if geometric.length() < 1e-6 {
                continue;
            }
            let averaged = normal(a) + normal(b) + normal(c);
            assert!(
                geometric.normalize().dot(averaged.normalize_or_zero()) > 0.0,
                "triangle {t} winds inward: geometric {geometric:?} vs shading {averaged:?}"
            );
        }
    }

    #[test]
    fn every_canonical_primitive_builds_a_usable_mesh() {
        for (name, primitive) in Primitive::CANONICAL {
            let mesh = primitive.build();
            assert!(mesh.vertex_count() >= 3, "{name} has no vertices");
            assert_eq!(mesh.index_count() % 3, 0, "{name} has a partial triangle");
            assert!(!mesh.aabb.is_empty(), "{name} has empty bounds");
            assert_unit_normals(&mesh);
            assert_uvs_in_unit_range(&mesh);
            assert_outward_facing(&mesh);
        }
    }

    /// Indices address real vertices. An out-of-range index is a GPU
    /// crash or garbage geometry, not a visual glitch.
    #[test]
    fn every_index_is_in_range() {
        for (name, primitive) in Primitive::CANONICAL {
            let mesh = primitive.build();
            let count = mesh.vertex_count();
            for &i in &mesh.indices {
                assert!(i < count, "{name} index {i} exceeds {count} vertices");
            }
        }
    }

    /// Building the same recipe twice gives the same mesh — the baked
    /// assets have to be reproducible, or their GUIDs churn.
    #[test]
    fn generation_is_deterministic() {
        for (name, primitive) in Primitive::CANONICAL {
            let a = primitive.build();
            let b = primitive.build();
            assert_eq!(a.indices, b.indices, "{name} indices differ between runs");
            let positions =
                |m: &Mesh| -> Vec<[f32; 3]> { m.vertices.iter().map(|v| v.position).collect() };
            assert_eq!(positions(&a), positions(&b), "{name} positions differ");
        }
    }
}
