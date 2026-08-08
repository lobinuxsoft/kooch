use super::*;
use kooch_ecs::reflect::Reflect;

#[test]
fn default_is_active_priority_zero() {
    let f = StreamingFocus::default();
    assert!(f.active);
    assert_eq!(f.priority, 0);
}

#[test]
fn reflect_exposes_fields() {
    let f = StreamingFocus::default();
    let names: Vec<&str> = f.reflect_fields().iter().map(|x| x.name).collect();
    assert_eq!(names, &["active", "priority"]);
}
