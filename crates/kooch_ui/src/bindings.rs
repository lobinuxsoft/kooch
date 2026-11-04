//! `bind:value="Type.field"` (#280 phase 6): a control kept in step with a field of a component on
//! the document's entity, through reflection. The UI holds no state of its own: the field does.

use std::any::TypeId;

use kooch_core::resource::Resources;
use kooch_ecs::ReflectValue;
use kooch_ecs::component::ComponentRegistry;

use crate::UiTrees;

/// After the pointer and focus: a gesture this frame writes the field, otherwise the field moves
/// the control. In `PreUpdate`, so gameplay reads what the player just set.
pub fn bind_values(resources: &mut Resources) {
    let mut bound = Vec::new();
    if let Some(trees) = resources.get_mut::<UiTrees>() {
        trees.for_each_entity_mut(|document, tree| {
            bound.extend(tree.take_bindings().into_iter().map(|b| (document, b)));
        });
    }
    if bound.is_empty() {
        return;
    }
    let mut reads = Vec::new();
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
    for (document, bound) in bound {
        let Some(type_id) = component_id(registry, &bound.component) else {
            kooch_core::warn_once!(
                "`bind:value` names `{}`, which is no reflected component",
                bound.component
            );
            continue;
        };
        let Some(field) = registry
            .reflect_get_fields(&type_id, document)
            .and_then(|fields| fields.into_iter().find(|(name, _)| *name == *bound.field))
            .map(|(_, value)| value)
        else {
            continue;
        };
        if bound.touched {
            if let Some(value) = field_written(&field, bound.value)
                && let Err(error) =
                    registry.reflect_set_field(&type_id, document, &bound.field, value)
            {
                kooch_core::warn_once!("`bind:value` could not write: {error:?}");
            }
        } else if let Some(value) = field_number(&field) {
            reads.push((document, bound.node, value));
        }
    }
    let Some(trees) = resources.get_mut::<UiTrees>() else {
        return;
    };
    for (document, node, value) in reads {
        if let Some(tree) = trees.get_mut(document) {
            tree.set_value(node, value);
        }
    }
}

/// A reflected component by its name as written: `Hud` or its full path.
fn component_id(registry: &ComponentRegistry, name: &str) -> Option<TypeId> {
    registry
        .reflected_type_names()
        .into_iter()
        .find(|(_, full)| *full == name || full.rsplit("::").next() == Some(name))
        .map(|(id, _)| id)
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
