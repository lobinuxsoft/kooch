use super::*;

#[test]
fn default_is_zero() {
    let l = LodForceLevel::default();
    assert_eq!(l.level, 0);
}

#[test]
fn reflect_field_exposed() {
    let l = LodForceLevel { level: 3 };
    let fields = l.reflect_fields();
    let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
    assert_eq!(names, &["level"]);
}
