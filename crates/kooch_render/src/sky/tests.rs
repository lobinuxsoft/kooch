use super::*;
use crate::material::{Shader, ShaderKind};
use crate::meshlet::validate_sky;

#[test]
fn the_default_sky_compiles() {
    let shader = Shader::parse(DEFAULT_SKY_SHADER).expect("the engine's sky parses");
    assert_eq!(shader.kind, ShaderKind::Sky);
    validate_sky(&shader.params_wgsl(), &shader.source).expect("the engine's sky validates");
}
