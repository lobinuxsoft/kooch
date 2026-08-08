use super::*;

#[test]
fn interning_the_same_name_yields_the_same_id() {
    let mut names = ComponentNames::new();
    let a = names.intern("game::Health");
    let b = names.intern("game::Health");
    assert_eq!(a, b);
    assert_eq!(names.len(), 1);
}

#[test]
fn distinct_names_get_distinct_ids() {
    let mut names = ComponentNames::new();
    let a = names.intern("game::Health");
    let b = names.intern("game::Speed");
    assert_ne!(a, b);
    assert_eq!(names.name(a), Some("game::Health"));
    assert_eq!(names.name(b), Some("game::Speed"));
}

#[test]
fn id_does_not_assign() {
    let mut names = ComponentNames::new();
    assert_eq!(names.id("game::Health"), None);
    let id = names.intern("game::Health");
    assert_eq!(names.id("game::Health"), Some(id));
}

#[test]
fn name_of_unknown_id_is_none() {
    let names = ComponentNames::new();
    assert_eq!(names.name(ComponentId(7)), None);
}
