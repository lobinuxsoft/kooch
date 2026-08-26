use super::*;

#[test]
fn register_and_lookup() {
    let mut reg = ResourceRegistry::new();
    reg.register::<i32>("core::i32");
    reg.register::<String>("std::String");

    assert_eq!(reg.get_type_id("core::i32"), Some(TypeId::of::<i32>()));
    assert_eq!(reg.get_type_id("std::String"), Some(TypeId::of::<String>()));
    assert_eq!(reg.get_type_id("unknown"), None);
}

#[test]
fn overwrite_existing() {
    let mut reg = ResourceRegistry::new();
    reg.register::<i32>("my_type");
    reg.register::<f32>("my_type");

    assert_eq!(reg.get_type_id("my_type"), Some(TypeId::of::<f32>()));
}

#[test]
fn len_and_empty() {
    let mut reg = ResourceRegistry::new();
    assert!(reg.is_empty());

    reg.register::<i32>("i32");
    assert_eq!(reg.len(), 1);
    assert!(!reg.is_empty());
}
