//! Test code for `post_process`, in its own file.

use kooch_core::Guid;

use super::PostProcess;
use crate::reflect::{FieldKind, Reflect, ReflectValue};

const MATERIAL: &str = "kooch_render::material::asset::Material";

fn asset(guid: Option<Guid>) -> ReflectValue {
    ReflectValue::AssetRef {
        guid,
        asset_type: MATERIAL.to_owned(),
    }
}

/// The stack reads as a list whose new items are material pickers — even when it is empty.
#[test]
fn the_stack_reads_as_a_list() {
    let post = PostProcess::default();
    let Some(ReflectValue::List { items, element }) = post.reflect_get("materials") else {
        panic!("not a list");
    };
    assert!(items.is_empty());
    assert_eq!(*element, asset(None));
    let meta = post
        .reflect_fields()
        .iter()
        .find(|meta| meta.name == "materials")
        .unwrap();
    assert_eq!(meta.kind, FieldKind::List);
}

/// A list written back sets every item, in order.
#[test]
fn a_list_sets_in_order() {
    let (a, b) = (Guid::new_v4(), Guid::new_v4());
    let mut post = PostProcess::default();
    post.reflect_set(
        "materials",
        ReflectValue::List {
            items: vec![asset(Some(b)), asset(Some(a))],
            element: Box::new(asset(None)),
        },
    )
    .unwrap();
    assert_eq!(post.materials, [Some(b), Some(a)]);
}

/// 🔴 A scene saved before the stack holds `material: AssetRef`. It must load as a stack of one,
/// not come back empty — a renamed field drops its value in silence otherwise.
#[test]
fn an_old_material_loads() {
    let guid = Guid::new_v4();
    let mut post = PostProcess::default();
    post.reflect_set("material", asset(Some(guid))).unwrap();
    assert_eq!(post.materials, [Some(guid)]);
}

#[test]
fn a_wrong_item_is_refused() {
    let mut post = PostProcess::default();
    let result = post.reflect_set(
        "materials",
        ReflectValue::List {
            items: vec![ReflectValue::F32(1.0)],
            element: Box::new(asset(None)),
        },
    );
    assert!(result.is_err());
}

/// Scenes are RON of `ReflectValue`, and a list has to come back as it went.
#[test]
fn a_list_survives_ron() {
    let list = ReflectValue::List {
        items: vec![asset(Some(Guid::new_v4())), asset(None)],
        element: Box::new(asset(None)),
    };
    let text = ron::to_string(&list).unwrap();
    assert_eq!(ron::from_str::<ReflectValue>(&text).unwrap(), list);
}
