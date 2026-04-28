//! ome_sdf — Signed Distance Field system for oh_my_engine.
//!
//! Hosts the WGSL primitive library consumed by the ray-marching
//! renderer and (eventually) by the physics broadphase, plus the
//! chunk-local sparse SDF voxel storage backend (issue #136).

pub mod sparse;

/// Source of `shaders/sdf_primitives.wgsl` embedded at compile time.
///
/// Downstream crates include this in their pipelines via `concat!` or
/// by pasting it into a larger shader module.
pub const SDF_PRIMITIVES_WGSL: &str = include_str!("../shaders/sdf_primitives.wgsl");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_parses() {
        let module = naga::front::wgsl::parse_str(SDF_PRIMITIVES_WGSL)
            .expect("sdf_primitives.wgsl should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("sdf_primitives.wgsl should validate");
    }

    #[test]
    fn shader_exposes_core_functions() {
        // Guard against accidental renames — downstream crates call these by name.
        let expected = [
            "sdf_sphere",
            "sdf_box",
            "sdf_rounded_box",
            "sdf_capsule_y",
            "sdf_capsule",
            "sdf_capped_cylinder",
            "sdf_torus",
            "sdf_plane",
            "sdf_plane_y",
            "sdf_union",
            "sdf_intersection",
            "sdf_subtraction",
            "sdf_smooth_union",
            "sdf_smooth_intersection",
            "sdf_smooth_subtraction",
            "transform_point",
            "scale_point",
            "sdf_normal_eps",
        ];
        for name in expected {
            assert!(
                SDF_PRIMITIVES_WGSL.contains(&format!("fn {name}(")),
                "missing function `{name}` in sdf_primitives.wgsl",
            );
        }
    }
}
