//! Sky renderer — procedural vertical-gradient background pass.
//!
//! Runs **before** the ray-march pass when an ECS entity with an active
//! `SkyRenderer` component exists. Clears the viewport color target and
//! writes `frag_depth = 1.0` so subsequent passes depth-test correctly.
//!
//! When no `SkyRenderer` is active the pass is skipped entirely and the
//! ray-march pass reverts to its internal gradient (matching pre-SkyRenderer
//! behavior).

mod renderer;

pub use renderer::{ActiveSky, SkyRenderPass};

/// Sky shader source (vertex + fragment, fullscreen triangle).
pub(crate) const SHADER_SOURCE: &str = include_str!("../../shaders/sky_main.wgsl");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_parses() {
        let module = naga::front::wgsl::parse_str(SHADER_SOURCE).expect("sky shader should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("sky shader should validate");
    }
}
