//! Ray-marching renderer — fullscreen fragment shader that sphere-traces
//! SDF components from the ECS with per-entity CSG blend support.

mod instance;
mod renderer;
mod update;

pub use instance::RayMarchParams;
pub use renderer::RayMarchRenderer;

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
