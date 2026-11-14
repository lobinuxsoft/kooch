use super::*;
use crate::reflect::Reflect;

#[test]
fn reflect_fields() {
    let s = SkyRenderer::default();
    let fields = s.reflect_fields();
    let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
    assert_eq!(names, &["active", "priority", "material"]);
}
