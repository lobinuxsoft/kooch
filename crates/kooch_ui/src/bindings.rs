//! `bind:value="Type.field"` (#280 phase 6): a control kept in step with a field of a component on
//! the document's entity, through reflection. The UI holds no state of its own: the field does.

use std::any::TypeId;

use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::{Entity, ReflectValue};

use crate::UiTrees;
use crate::controls::Binding;

/// One binding to settle this frame.
struct Bound {
    document: Entity,
    node: crate::UiNode,
    binding: Binding,
    value: f32,
    touched: bool,
}

/// After the pointer and focus: a gesture this frame writes the field, otherwise the field moves
/// the control. In `PreUpdate`, so gameplay reads what the player just set.
pub fn bind_values(resources: &mut Resources) {
    let mut bound = Vec::new();
    if let Some(trees) = resources.get_mut::<UiTrees>() {
        trees.for_each_entity_mut(|document, tree| {
            for control in &mut tree.controls {
                if let Some(binding) = &control.bind {
                    bound.push(Bound {
                        document,
                        node: control.node,
                        binding: binding.clone(),
                        value: control.value,
                        touched: control.touched,
                    });
                }
                control.touched = false;
            }
        });
    }
    if bound.is_empty() {
        return;
    }
    let mut reads = Vec::new();
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
    for bound in bound {
        let Some(type_id) = component_id(registry, &bound.binding.component) else {
            kooch_core::warn_once!(
                "`bind:value` names `{}`, which is no reflected component",
                bound.binding.component
            );
            continue;
        };
        let Some(field) = registry
            .reflect_get_fields(&type_id, bound.document)
            .and_then(|fields| {
                fields
                    .into_iter()
                    .find(|(name, _)| *name == *bound.binding.field)
            })
            .map(|(_, value)| value)
        else {
            continue;
        };
        if bound.touched {
            if let Some(value) = written(&field, bound.value)
                && let Err(error) = registry.reflect_set_field(
                    &type_id,
                    bound.document,
                    &bound.binding.field,
                    value,
                )
            {
                kooch_core::warn_once!("`bind:value` could not write: {error:?}");
            }
        } else if let Some(value) = number(&field) {
            reads.push((bound.document, bound.node, value));
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
fn number(value: &ReflectValue) -> Option<f32> {
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
fn written(field: &ReflectValue, value: f32) -> Option<ReflectValue> {
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
