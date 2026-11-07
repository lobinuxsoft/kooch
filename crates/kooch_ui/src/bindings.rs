//! `bind:value="Type.field"` (#280 phase 6): a control kept in step with a field of a component on
//! the document's entity, through reflection. The UI holds no state of its own: the field does.

use kooch_core::resource::Resources;
use kooch_ecs::ReflectValue;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;

use crate::{UiTree, UiTrees};

/// After the pointer and focus: a gesture this frame writes the field, otherwise the field moves
/// the control. In `PreUpdate`, so gameplay reads what the player just set.
pub fn bind_values(resources: &mut Resources) {
    with_trees(resources, |document, tree, registry| {
        for index in 0..tree.controls.len() {
            let Some(control) = tree.controls.get_mut(index) else {
                continue;
            };
            let touched = std::mem::take(&mut control.touched);
            let (node, value) = (control.node, control.value);
            let Some(path) = control.bind.as_mut() else {
                continue;
            };
            let Some(type_id) = path.resolve(registry) else {
                continue;
            };
            let Some(field) = registry.reflect_get_field(&type_id, document, &path.field) else {
                continue;
            };
            if touched {
                if let Some(value) = field_written(&field, value)
                    && let Err(error) =
                        registry.reflect_set_field(&type_id, document, &path.field, value)
                {
                    kooch_core::warn_once!("`bind:value` could not write: {error:?}");
                }
            } else if let Some(value) = field_number(&field) {
                tree.set_value(node, value);
            }
        }
    });
}

/// `bind:param.<name>`: the field moves the parameter, never the other way. Costs a tree with no
/// such binding nothing.
pub fn bind_params(resources: &mut Resources) {
    with_trees(resources, |document, tree, registry| {
        for index in 0..tree.param_binds.len() {
            let Some(bind) = tree.param_binds.get_mut(index) else {
                continue;
            };
            let value = bind.path.resolve(registry).and_then(|type_id| {
                registry.reflect_get_field(&type_id, document, &bind.path.field)
            });
            if let Some(value) = value.as_ref().and_then(field_number) {
                let (node, param) = (bind.node, bind.param.clone());
                tree.set_param(node, &param, value);
            }
        }
    });
}

/// A label's `{Type.field}` holes: the text is formatted and laid out again only when a value
/// it shows moved.
pub fn bind_texts(resources: &mut Resources) {
    with_trees(resources, |document, tree, registry| {
        for index in 0..tree.text_binds.len() {
            let Some(bind) = tree.text_binds.get_mut(index) else {
                continue;
            };
            let mut moved = false;
            for (path, shown) in bind.holes.iter_mut().zip(bind.shown.iter_mut()) {
                let value = path.resolve(registry).and_then(|type_id| {
                    registry.reflect_get_field(&type_id, document, &path.field)
                });
                if value != *shown {
                    *shown = value;
                    moved = true;
                }
            }
            if moved && let Some(text) = bind.format() {
                let node = bind.node;
                tree.write_text(node, &text);
            }
        }
    });
}

/// Every document's tree beside the registry. The trees are lifted out for the walk, which swaps
/// two maps and allocates nothing; `UiTrees` reads empty only while this runs.
fn with_trees(
    resources: &mut Resources,
    mut each: impl FnMut(Entity, &mut UiTree, &mut ComponentRegistry),
) {
    let Some(mut trees) = resources.get_mut::<UiTrees>().map(std::mem::take) else {
        return;
    };
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        trees.for_each_entity_mut(|document, tree| each(document, tree, registry));
    }
    if let Some(slot) = resources.get_mut::<UiTrees>() {
        *slot = trees;
    }
}

/// A field a control can show.
pub fn field_number(value: &ReflectValue) -> Option<f32> {
    Some(match *value {
        ReflectValue::F32(v) => v,
        ReflectValue::F64(v) => v as f32,
        ReflectValue::Bool(v) => v as u8 as f32,
        ReflectValue::U8(v) => v as f32,
        ReflectValue::U16(v) => v as f32,
        ReflectValue::U32(v) => v as f32,
        ReflectValue::I8(v) => v as f32,
        ReflectValue::I16(v) => v as f32,
        ReflectValue::I32(v) => v as f32,
        _ => return None,
    })
}

/// `value` in the field's own type.
pub fn field_written(field: &ReflectValue, value: f32) -> Option<ReflectValue> {
    let whole = value.round();
    Some(match field {
        ReflectValue::F32(_) => ReflectValue::F32(value),
        ReflectValue::F64(_) => ReflectValue::F64(value as f64),
        ReflectValue::Bool(_) => ReflectValue::Bool(value > 0.5),
        ReflectValue::U8(_) => ReflectValue::U8(whole as u8),
        ReflectValue::U16(_) => ReflectValue::U16(whole as u16),
        ReflectValue::U32(_) => ReflectValue::U32(whole as u32),
        ReflectValue::I8(_) => ReflectValue::I8(whole as i8),
        ReflectValue::I16(_) => ReflectValue::I16(whole as i16),
        ReflectValue::I32(_) => ReflectValue::I32(whole as i32),
        _ => return None,
    })
}
