//! Re-export of the shared `primitive_aabb` helper that now lives in
//! [`ome_bvh::sdf_primitive`]. Hoisted alongside [`SdfPrimitive`] so
//! every primitive producer (renderer ECS collector, world content
//! sources, future Edit Baker) computes the same leaf AABB for a given
//! primitive — divergence here drops silhouettes from the BVH cull.

pub use ome_bvh::sdf_primitive::primitive_aabb;
