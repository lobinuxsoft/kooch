//! Ray-marching renderer — fullscreen fragment shader that sphere-traces
//! SDF components from the ECS with per-entity CSG blend support.

mod aabb;
mod bvh;
mod instance;
mod renderer;
mod update;

use ome_core::resource::Resources;
use ome_ecs::query::Query;
use ome_ecs::{SdfBox, SdfCapsule, SdfCylinder, SdfPlane, SdfSphere, SdfTorus};

pub use instance::RayMarchParams;
pub use renderer::RayMarchRenderer;

/// Returns `true` when at least one SDF primitive entity in the scene
/// has its `visible` flag set. Used by render systems as a gate to
/// decide whether the ray-march pass needs to run.
///
/// Iterates **every** built-in SDF primitive type (`SdfSphere`,
/// `SdfBox`, `SdfCapsule`, `SdfCylinder`, `SdfTorus`, `SdfPlane`) so
/// the gate stays in sync with the upload path in
/// [`update::RayMarchRenderer::update_scene`]. If a new SDF primitive
/// is added, both this function **and** the collect path in
/// `update.rs` need to be updated together.
pub fn has_any_visible_sdf(resources: &Resources) -> bool {
    macro_rules! any_visible {
        ($ty:ty) => {{
            let q = Query::<&$ty>::new(resources);
            let mut found = false;
            q.for_each(|s| {
                if s.visible {
                    found = true;
                }
            });
            found
        }};
    }
    any_visible!(SdfSphere)
        || any_visible!(SdfBox)
        || any_visible!(SdfCapsule)
        || any_visible!(SdfCylinder)
        || any_visible!(SdfTorus)
        || any_visible!(SdfPlane)
}

/// Fullscreen shader source (primitives library + ray-march main),
/// concatenated at compile time.
const SHADER_SOURCE: &str = concat!(
    include_str!("../../../ome_sdf/shaders/sdf_primitives.wgsl"),
    "\n",
    include_str!("../../shaders/raymarch_main.wgsl"),
);

/// Pool-driven scene-SDF library — `eval_scene_bvh` + `descend_blas`
/// + the OmeAccel pool bindings (group 1, bindings 5..=10). NO entry
/// point: concatenate AFTER `sdf_primitives.wgsl` and BEFORE either
/// the fragment-shader entry in `raymarch_main.wgsl` (production
/// path) or the compute-shader entry in `raymarch_pool_smoke.wgsl`
/// (smoke test).
pub const POOL_EVAL_LIBRARY_WGSL: &str = include_str!("../../shaders/raymarch_pool_eval.wgsl");

/// Standalone compute-kernel smoke test: `sdf_primitives` + pool
/// library + `cs_eval_smoke`. Drives `eval_scene_bvh` over a caller-
/// provided sample-point buffer; consumed by
/// `tests/pool_eval_smoke.rs` to validate the pool shader in
/// isolation from the renderer pipeline.
pub const POOL_EVAL_SHADER_SOURCE: &str = concat!(
    include_str!("../../../ome_sdf/shaders/sdf_primitives.wgsl"),
    "\n",
    include_str!("../../shaders/raymarch_pool_eval.wgsl"),
    "\n",
    include_str!("../../shaders/raymarch_pool_smoke.wgsl"),
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_parses() {
        let module = naga::front::wgsl::parse_str(SHADER_SOURCE)
            .expect("concatenated raymarch shader should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("concatenated raymarch shader should validate");
    }

    /// Pool-driven shader (#360): naga must parse the concatenated
    /// source. PR-2 wires it into the fragment pipeline; this test
    /// pins the WGSL so the contract survives until then.
    #[test]
    fn pool_eval_shader_parses() {
        let module = naga::front::wgsl::parse_str(POOL_EVAL_SHADER_SOURCE)
            .expect("concatenated pool-eval shader should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("concatenated pool-eval shader should validate");
    }

    /// `eval_scene_bvh` and the smoke-test compute entry point must
    /// both survive parsing + validation. Pinning their presence here
    /// catches accidental renames in PR-2 before the integration
    /// tests do.
    #[test]
    fn pool_eval_shader_exposes_required_entry_points() {
        let module = naga::front::wgsl::parse_str(POOL_EVAL_SHADER_SOURCE)
            .expect("pool-eval shader should parse");
        let function_names: Vec<&str> = module
            .functions
            .iter()
            .filter_map(|(_, f)| f.name.as_deref())
            .collect();
        assert!(
            function_names.iter().any(|n| *n == "eval_scene_bvh"),
            "pool-eval shader must expose `eval_scene_bvh`; saw {function_names:?}"
        );
        assert!(
            function_names.iter().any(|n| *n == "descend_blas"),
            "pool-eval shader must expose `descend_blas` (BLAS traversal); saw {function_names:?}"
        );
        let entry_names: Vec<&str> = module
            .entry_points
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert!(
            entry_names.iter().any(|n| *n == "cs_eval_smoke"),
            "pool-eval shader must expose `cs_eval_smoke` compute entry point; saw {entry_names:?}"
        );
    }
}
