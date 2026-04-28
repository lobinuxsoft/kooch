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
}
