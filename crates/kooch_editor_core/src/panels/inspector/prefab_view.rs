//! Editing a prefab in the Inspector.
//!
//! # What makes this different from inspecting an entity
//!
//! A prefab's entities do not exist. There is no `Entity` to name, no
//! storage to read a component out of, and nothing for an edit to be
//! applied to — the data is a document, and a field is a `ReflectValue`
//! sitting in a `Vec` inside it.
//!
//! Everything *below* that difference is the same, which is why this
//! renders through [`super::single::draw_reflected_fields`] rather than
//! reimplementing it: the same grid, the same asset and entity pickers,
//! the same `shown_when` rules that hide a capsule's half-height while a
//! sphere is selected. A second copy of those rules is a second copy to
//! keep correct, and the one not exercised by the panel people use every
//! day is the one that rots.
//!
//! # Why edits are live before they are saved
//!
//! They land in `Assets<SceneDocument>`, which is also the cache
//! `spawn_prefab` reads. So anything spawned after an edit gets the edited
//! values while the file still holds the old ones. That is the cost of an
//! explicit save, and the reason the button says so out loud instead of
//! relying on the user to remember.

use std::collections::HashMap;

use glam::Vec3;
use kooch_core::Guid;
use kooch_ecs::entity::Entity;

use crate::actions::EditorAction;
use crate::icons;
use crate::state::ReflectedTypeInfo;

use super::RotationContext;
use super::asset_view::{PrefabComponentView, PrefabDetail, PrefabEntityView};
use super::{AssetCatalogEntry, EntityDisplayInfo};
use crate::state::EulerCacheKey;

/// Generation no live entity carries.
///
/// The euler-angle cache is keyed by entity, and a prefab has none. Using
/// the document index alone would collide with a real entity's cache entry
/// and make a rotation widget jump while being dragged; this keeps the two
/// spaces apart without threading a second key type through the grid.
const PREFAB_PSEUDO_GENERATION: u32 = u32::MAX;

/// Renders a prefab's entities and their components, and the Save button.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_prefab_inspector(
    ui: &mut egui::Ui,
    guid: Guid,
    detail: &PrefabDetail,
    euler_cache: &mut HashMap<EulerCacheKey, Vec3>,
    asset_catalog: &[AssetCatalogEntry],
    entities: &[EntityDisplayInfo],
    reflected_types: &[ReflectedTypeInfo],
    actions: &mut Vec<EditorAction>,
) {
    draw_save_bar(ui, guid, detail, actions);
    ui.separator();

    if detail.entities.is_empty() {
        ui.weak("(this prefab describes no entities)");
        return;
    }

    for entity in &detail.entities {
        draw_entity_section(
            ui,
            guid,
            entity,
            euler_cache,
            asset_catalog,
            entities,
            reflected_types,
            actions,
        );
    }
}

/// The Save button and what it is telling the user.
fn draw_save_bar(
    ui: &mut egui::Ui,
    guid: Guid,
    detail: &PrefabDetail,
    actions: &mut Vec<EditorAction>,
) {
    ui.horizontal(|ui| {
        // Disabled rather than hidden: a button that appears only when
        // there is something to save gives no clue where saving lives the
        // rest of the time.
        let save = ui.add_enabled(
            detail.dirty,
            egui::Button::new(format!("{} Save prefab", icons::PACKAGE)),
        );
        if save.clicked() {
            actions.push(EditorAction::SavePrefabAsset(guid));
        }
        match detail.dirty {
            true => {
                ui.label("Unsaved changes").on_hover_text(
                    "Anything spawned now already uses these values; the file does not.",
                );
            }
            false => {
                ui.weak("Saved");
            }
        }
    });
}

/// One entity of the prefab, with its components.
#[allow(clippy::too_many_arguments)]
fn draw_entity_section(
    ui: &mut egui::Ui,
    guid: Guid,
    entity: &PrefabEntityView,
    euler_cache: &mut HashMap<EulerCacheKey, Vec3>,
    asset_catalog: &[AssetCatalogEntry],
    entities: &[EntityDisplayInfo],
    reflected_types: &[ReflectedTypeInfo],
    actions: &mut Vec<EditorAction>,
) {
    let title = match entity.is_root {
        true => format!("{} {}  (root)", icons::PACKAGE, entity.name),
        false => format!("{} {}", icons::CUBE, entity.name),
    };

    egui::CollapsingHeader::new(title)
        .id_salt(("prefab_entity", entity.index))
        .default_open(entity.is_root)
        .show(ui, |ui| {
            for component in &entity.components {
                draw_component_section(
                    ui,
                    guid,
                    entity.index,
                    component,
                    euler_cache,
                    asset_catalog,
                    entities,
                    actions,
                );
            }
            ui.add_space(4.0);
            draw_add_component(ui, guid, entity, reflected_types, actions);
        });
}

/// One component's fields, plus its remove button.
#[allow(clippy::too_many_arguments)]
fn draw_component_section(
    ui: &mut egui::Ui,
    guid: Guid,
    entity_index: usize,
    component: &PrefabComponentView,
    euler_cache: &mut HashMap<EulerCacheKey, Vec3>,
    asset_catalog: &[AssetCatalogEntry],
    entities: &[EntityDisplayInfo],
    actions: &mut Vec<EditorAction>,
) {
    let name = component.short_name.clone();
    // Built the same way the entity inspector builds a section — a bold
    // title beside the same glyph, and removal as a small X in the header
    // rather than a button under the fields. They are the same panel
    // showing the same kind of thing, and looking almost-alike is worse
    // than looking different: it reads as a second implementation, which
    // is exactly what it would have become.
    //
    // Keyed on the entity index as well as the type: unlike an entity's
    // sections, several of these are on screen at once and two entities of
    // one prefab can carry the same component.
    let id = ui.make_persistent_id(format!(
        "prefab_comp_{entity_index}_{}",
        component.type_name
    ));
    let section =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
    section
        .show_header(ui, |ui| {
            ui.strong(format!("{} {name}", icons::PUZZLE_PIECE));
            // `Parent` is the hierarchy, not a component to take off by
            // hand — removing it would orphan a child inside a file whose
            // whole point is being one tree.
            if component.short_name != "Parent"
                && ui
                    .small_button(icons::X)
                    .on_hover_text("Remove component")
                    .clicked()
                && let Some(resolved) = component.resolved
            {
                actions.push(EditorAction::EditPrefabComponent {
                    prefab: guid,
                    entity_index,
                    component: resolved.component,
                    add: false,
                });
            }
        })
        .body(|ui| {
            let Some(resolved) = component.resolved else {
                // Genuinely unknown now: not in the reflected registry
                // and not declared by any loaded plugin. Naming the type
                // rather than saying "no type", because the useful next
                // question is *which* one — a scene written by a build
                // with a feature this one lacks, or a renamed crate.
                ui.weak(format!("Unknown component: {}", component.type_name));
                return;
            };

            if component.fields.is_empty() {
                ui.weak("(no fields)");
            } else {
                let edits = super::single::draw_reflected_fields(
                    ui,
                    Entity::new(entity_index as u32, PREFAB_PSEUDO_GENERATION),
                    resolved.type_id,
                    resolved.component,
                    &component.fields,
                    resolved.field_metas,
                    euler_cache,
                    // A prefab has no world transform to display against,
                    // so rotations are edited in local space only — which
                    // is what a prefab's transform means anyway.
                    RotationContext::local_only(),
                    asset_catalog,
                    entities,
                );
                // A prefab's edits go to its document.
                for (field, value) in edits {
                    actions.push(EditorAction::EditPrefabField {
                        prefab: guid,
                        entity_index,
                        component: component.type_name.clone(),
                        field,
                        value,
                    });
                }
            }
        });
}

/// The Add Component menu for one of the prefab's entities.
fn draw_add_component(
    ui: &mut egui::Ui,
    guid: Guid,
    entity: &PrefabEntityView,
    reflected_types: &[ReflectedTypeInfo],
    actions: &mut Vec<EditorAction>,
) {
    let present: Vec<&str> = entity
        .components
        .iter()
        .map(|c| c.short_name.as_str())
        .collect();
    let available: Vec<&ReflectedTypeInfo> = reflected_types
        .iter()
        .filter(|t| !present.contains(&t.short_name.as_str()))
        .collect();
    if available.is_empty() {
        return;
    }

    ui.menu_button(format!("{} Add Component", icons::PLUS), |ui| {
        crate::panels::add_component_menu::draw_categorized(ui, &available, |component| {
            // The menu speaks `ComponentId` and the document speaks type
            // names, because a name outlives the process that wrote it.
            // Translating needs the registry, so the handler does it —
            // it has a world and this does not.
            actions.push(EditorAction::EditPrefabComponent {
                prefab: guid,
                entity_index: entity.index,
                component,
                add: true,
            });
        });
    });
}
