use super::*;
use crate::post_process::PostEffect;

/// A field the struct no longer has is skipped, so a scene from another layout still loads.
#[test]
fn an_unknown_field_is_skipped() {
    let value = ReflectValue::Struct(vec![
        ("weight".to_owned(), ReflectValue::F32(0.5)),
        ("gone".to_owned(), ReflectValue::Bool(true)),
    ]);
    let list = ReflectValue::List {
        items: vec![value],
        element: Box::new(struct_value(&PostEffect::default())),
    };
    let effects: Vec<PostEffect> = list_from(list, None, "effects").unwrap();
    assert_eq!(effects[0].weight, 0.5);
    assert!(effects[0].enabled, "an absent field keeps its default");
}

/// Without a `bare` field, a plain value is not an item.
#[test]
fn a_plain_item_needs_bare() {
    let list = ReflectValue::List {
        items: vec![ReflectValue::F32(1.0)],
        element: Box::new(ReflectValue::Struct(Vec::new())),
    };
    assert!(list_from::<PostEffect>(list, None, "effects").is_err());
}
