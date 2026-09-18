//! Test code for `post_process`, in its own file.

use kooch_core::Guid;

use super::{PostEffect, PostProcess};
use crate::reflect::{FieldKind, Reflect, ReflectValue, struct_value};

const MATERIAL: &str = "kooch_render::material::asset::Material";

fn asset(guid: Option<Guid>) -> ReflectValue {
    ReflectValue::AssetRef {
        guid,
        asset_type: MATERIAL.to_owned(),
    }
}

fn effect(guid: Guid, enabled: bool, weight: f32) -> PostEffect {
    PostEffect {
        material: Some(guid),
        enabled,
        weight,
    }
}

/// The stack reads as a list of structs whose new item is a default effect, even when empty, and
/// whose field metadata carries the weight's slider range.
#[test]
fn the_stack_reads_as_structs() {
    let post = PostProcess::default();
    let Some(ReflectValue::List { items, element }) = post.reflect_get("effects") else {
        panic!("not a list");
    };
    assert!(items.is_empty());
    assert_eq!(*element, struct_value(&PostEffect::default()));
    let meta = post
        .reflect_fields()
        .iter()
        .find(|meta| meta.name == "effects")
        .unwrap();
    assert_eq!(meta.kind, FieldKind::List);
    let weight = meta.fields.iter().find(|f| f.name == "weight").unwrap();
    assert_eq!(weight.range.map(|r| (r.min, r.max)), Some((0.0, 1.0)));
}

/// A list written back sets every item and every field, in order.
#[test]
fn a_list_sets_in_order() {
    let (a, b) = (Guid::new_v4(), Guid::new_v4());
    let effects = vec![effect(b, false, 0.25), effect(a, true, 1.0)];
    let mut source = PostProcess::default();
    source.effects = effects.clone();
    let mut post = PostProcess::default();
    post.reflect_set("effects", source.reflect_get("effects").unwrap())
        .unwrap();
    assert_eq!(post.effects, effects);
}

/// 🔴 A scene saved before #1209 holds `materials: [AssetRef, ...]`. Each must load as an effect
/// on at full weight, not come back empty — a renamed field drops its value in silence otherwise.
#[test]
fn an_asset_list_loads() {
    let (a, b) = (Guid::new_v4(), Guid::new_v4());
    let mut post = PostProcess::default();
    post.reflect_set(
        "materials",
        ReflectValue::List {
            items: vec![asset(Some(a)), asset(Some(b))],
            element: Box::new(asset(None)),
        },
    )
    .unwrap();
    assert_eq!(post.effects, [effect(a, true, 1.0), effect(b, true, 1.0)]);
}

/// Older still: a single `material` before the stack existed loads as a stack of one.
#[test]
fn an_old_material_loads() {
    let guid = Guid::new_v4();
    let mut post = PostProcess::default();
    post.reflect_set("material", asset(Some(guid))).unwrap();
    assert_eq!(post.effects, [effect(guid, true, 1.0)]);
}

/// Something that is not an effect or a material is refused, not silently dropped.
#[test]
fn a_wrong_item_is_refused() {
    let mut post = PostProcess::default();
    let result = post.reflect_set(
        "effects",
        ReflectValue::List {
            items: vec![ReflectValue::F32(1.0)],
            element: Box::new(asset(None)),
        },
    );
    assert!(result.is_err());
}

/// An effect off, at zero weight, or without a material draws nothing.
#[test]
fn only_live_effects_draw() {
    let guid = Guid::new_v4();
    assert_eq!(effect(guid, true, 0.5).drawn(), Some(guid));
    assert_eq!(effect(guid, false, 1.0).drawn(), None);
    assert_eq!(effect(guid, true, 0.0).drawn(), None);
    assert_eq!(PostEffect::default().drawn(), None);
}

/// Scenes are RON of `ReflectValue`, and a list of structs has to come back as it went.
#[test]
fn a_struct_list_survives_ron() {
    let mut post = PostProcess::default();
    post.effects = vec![effect(Guid::new_v4(), false, 0.5), PostEffect::default()];
    let list = post.reflect_get("effects").unwrap();
    let text = ron::to_string(&list).unwrap();
    assert_eq!(ron::from_str::<ReflectValue>(&text).unwrap(), list);
}
