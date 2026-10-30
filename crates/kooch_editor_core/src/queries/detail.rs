//! The selected entities' values, re-read on frames where the rest of the snapshot is reused.

use std::collections::HashSet;

use kooch_core::resource::Resources;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::component::{ComponentNames, ComponentRegistry, DynamicTypeRegistry};
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::hierarchy::GlobalTransform;

use crate::state::{ComponentDisplayInfo, EntityDisplayInfo};

use super::{ArchetypeMeta, display_order, parked_components, reflected_fields};

/// Re-reads the detail for `detail_for` into rows gathered without it.
///
/// 🔴 This is what lets the snapshot be cached at all: the Inspector shows values that move every
/// frame while the game runs, but only for what is selected — one entity, not four thousand.
pub(crate) fn refresh_detail(
    resources: &Resources,
    rows: &mut [EntityDisplayInfo],
    detail_for: &HashSet<kooch_ecs::Entity>,
) {
    if detail_for.is_empty() {
        return;
    }
    let Some(archetypes) = resources.get::<ArchetypeRegistry>() else {
        return;
    };
    let components = resources.get::<ComponentRegistry>();
    let names = resources.get::<ComponentNames>();
    let dynamic = resources.get::<DynamicComponents>();
    let editable = resources
        .get::<crate::remote_session::RemoteState>()
        .is_some_and(|state| state.is_connected())
        || resources
            .get::<DynamicTypeRegistry>()
            .is_some_and(|types| !types.is_empty());

    let rotation = |entity: kooch_ecs::Entity| {
        components
            .as_ref()
            .and_then(|registry| registry.get_cpu::<GlobalTransform>())
            .and_then(|storage| storage.get(entity))
            .map(|transform| transform.rotation())
    };
    // A `PositionOnly` child's rotation is already in world axes.
    let rides = |entity: kooch_ecs::Entity| {
        components
            .as_ref()
            .and_then(|registry| registry.get_cpu::<kooch_ecs::hierarchy::PositionOnly>())
            .is_some_and(|storage| storage.get(entity).is_some())
    };

    for row in rows.iter_mut() {
        if !detail_for.contains(&row.entity) {
            continue;
        }
        row.global_rotation = rotation(row.entity);
        row.parent_global_rotation = match rides(row.entity) {
            true => row.parent.map(|_| glam::Quat::IDENTITY),
            false => row.parent.and_then(rotation),
        };

        let Some(archetype) = archetypes
            .entity_archetype(row.entity)
            .and_then(|id| archetypes.get(id))
        else {
            continue;
        };
        let meta = ArchetypeMeta::resolve(archetype, components.as_deref(), names.as_deref());
        let mut comps: Vec<ComponentDisplayInfo> = meta
            .components
            .iter()
            .map(|component| ComponentDisplayInfo {
                type_id: component.type_id,
                component: component.component,
                short_name: std::borrow::Cow::Borrowed(component.short_name),
                fields: reflected_fields(components.as_deref(), component, row.entity, true),
                field_metas: component.field_metas,
                visibility: component.visibility,
            })
            .collect();
        if let Some(parked) = dynamic
            .as_ref()
            .map(|dynamic| parked_components(dynamic, names.as_deref(), row.entity, editable, true))
            .filter(|parked| !parked.is_empty())
        {
            comps.extend(parked);
            comps.sort_by(|a, b| display_order(&a.short_name, &b.short_name));
        }
        row.components = comps;
    }
}
