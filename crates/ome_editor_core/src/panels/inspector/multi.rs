//! Multi-entity inspector — merged component view across selection.

use std::any::TypeId;
use std::collections::{HashMap, HashSet};

use ome_ecs::entity::Entity;
use ome_ecs::reflect::{FieldMeta, InspectorVisibility, ReflectValue};

use crate::actions::EditorAction;
use crate::icons;
use crate::state::{EntityDisplayInfo, ReflectedTypeInfo};

use super::widgets::{choices_for, draw_readonly_value, draw_value_widget};

/// A field value across multiple selected entities.
pub(super) enum MultiFieldValue {
    /// All selected entities share the same value.
    Uniform(ReflectValue),
    /// Values differ — stores the first entity's value as DragValue base.
    Mixed(ReflectValue),
}

/// Merged component info across multiple selected entities.
pub(super) struct MultiComponentInfo {
    pub(super) type_id: TypeId,
    pub(super) short_name: String,
    /// How many of the selected entities have this component.
    pub(super) present_count: usize,
    /// Total number of selected entities.
    pub(super) total_count: usize,
    /// Merged fields (`None` = no reflection).
    pub(super) fields: Option<Vec<(String, MultiFieldValue)>>,
    /// Static field metadata parallel to `fields`. Shared across entities
    /// because the reflection layout is the same per component type.
    pub(super) field_metas: Option<&'static [FieldMeta]>,
    pub(super) visibility: InspectorVisibility,
}

/// Collects shared component data across `selected` entities.
pub(super) fn gather_multi_component_info(
    entities: &[EntityDisplayInfo],
    selected: &[Entity],
) -> Vec<MultiComponentInfo> {
    let selected_set: HashSet<Entity> = selected.iter().copied().collect();
    let selected_infos: Vec<&EntityDisplayInfo> = entities
        .iter()
        .filter(|e| selected_set.contains(&e.entity))
        .collect();

    if selected_infos.is_empty() {
        return Vec::new();
    }

    let total_count = selected_infos.len();

    // Count how many selected entities have each component and collect metadata.
    struct CompMeta {
        short_name: String,
        count: usize,
        visibility: InspectorVisibility,
        field_metas: Option<&'static [FieldMeta]>,
    }
    let mut comp_map: HashMap<TypeId, CompMeta> = HashMap::new();

    for info in &selected_infos {
        for comp in &info.components {
            let entry = comp_map.entry(comp.type_id).or_insert(CompMeta {
                short_name: comp.short_name.clone(),
                count: 0,
                visibility: comp.visibility,
                field_metas: comp.field_metas,
            });
            entry.count += 1;
        }
    }

    // Build result, excluding Hidden components.
    let mut result: Vec<MultiComponentInfo> = Vec::new();

    for (type_id, meta) in &comp_map {
        if meta.visibility == InspectorVisibility::Hidden {
            continue;
        }

        // Collect field lists from all selected entities that have this component.
        let field_lists: Vec<&Vec<(String, ReflectValue)>> = selected_infos
            .iter()
            .filter_map(|info| {
                info.components
                    .iter()
                    .find(|c| c.type_id == *type_id)
                    .and_then(|c| c.fields.as_ref())
            })
            .collect();

        let fields = if field_lists.is_empty() {
            // No entity has reflection for this component.
            None
        } else if field_lists.len() < meta.count {
            // Some entities have reflection, some don't — treat as no reflection.
            None
        } else {
            // Merge fields: compare values across all entities.
            let first = &field_lists[0];
            let merged: Vec<(String, MultiFieldValue)> = first
                .iter()
                .map(|(name, first_val)| {
                    let all_equal = field_lists[1..]
                        .iter()
                        .all(|fields| {
                            fields
                                .iter()
                                .find(|(n, _)| n == name)
                                .is_some_and(|(_, v)| v == first_val)
                        });
                    let multi_val = if all_equal {
                        MultiFieldValue::Uniform(first_val.clone())
                    } else {
                        MultiFieldValue::Mixed(first_val.clone())
                    };
                    (name.clone(), multi_val)
                })
                .collect();
            Some(merged)
        };

        result.push(MultiComponentInfo {
            type_id: *type_id,
            short_name: meta.short_name.clone(),
            present_count: meta.count,
            total_count,
            fields,
            field_metas: meta.field_metas,
            visibility: meta.visibility,
        });
    }

    result.sort_by(|a, b| a.short_name.cmp(&b.short_name));
    result
}

/// Returns the subset of `selected` entities that have a component with `type_id`.
pub(super) fn selected_entities_with_component(
    entities: &[EntityDisplayInfo],
    selected: &[Entity],
    type_id: TypeId,
) -> Vec<Entity> {
    let selected_set: HashSet<Entity> = selected.iter().copied().collect();
    entities
        .iter()
        .filter(|e| selected_set.contains(&e.entity))
        .filter(|e| e.components.iter().any(|c| c.type_id == type_id))
        .map(|e| e.entity)
        .collect()
}

/// Draws the multi-entity inspector when more than one entity is selected.
pub(super) fn draw_multi_entity_inspector(
    ui: &mut egui::Ui,
    entities: &[EntityDisplayInfo],
    selected: &[Entity],
    reflected_types: &[ReflectedTypeInfo],
    actions: &mut Vec<EditorAction>,
) {
    ui.label(format!("{} entities selected", selected.len()));
    ui.separator();

    let multi_info = gather_multi_component_info(entities, selected);

    // "Add Component" dropdown — show types not present on ALL selected entities.
    let all_have: HashSet<TypeId> = multi_info
        .iter()
        .filter(|c| c.present_count == c.total_count)
        .map(|c| c.type_id)
        .collect();
    let available: Vec<&ReflectedTypeInfo> = reflected_types
        .iter()
        .filter(|t| !all_have.contains(&t.type_id))
        .collect();

    if !available.is_empty() {
        ui.menu_button(format!("{} Add Component", icons::PLUS), |ui| {
            crate::panels::add_component_menu::draw_categorized(ui, &available, |type_id| {
                let have_it: HashSet<Entity> =
                    selected_entities_with_component(entities, selected, type_id)
                        .into_iter()
                        .collect();
                for &entity in selected {
                    if !have_it.contains(&entity) {
                        actions.push(EditorAction::AddComponent { entity, type_id });
                    }
                }
            });
        });
        ui.separator();
    }

    if multi_info.is_empty() {
        ui.weak("(no shared components)");
        return;
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        for comp in &multi_info {
            let is_read_only = comp.visibility == InspectorVisibility::ReadOnly;
            let id =
                ui.make_persistent_id(format!("multi_comp_{:?}", comp.type_id));

            egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                true,
            )
            .show_header(ui, |ui| {
                let label = if comp.present_count < comp.total_count {
                    format!(
                        "{} {} ({}/{})",
                        icons::PUZZLE_PIECE,
                        &comp.short_name,
                        comp.present_count,
                        comp.total_count
                    )
                } else {
                    format!("{} {}", icons::PUZZLE_PIECE, &comp.short_name)
                };
                ui.strong(label);

                // Removal is always available regardless of visibility:
                // `ReadOnly` gates field edits, not component lifecycle.
                if ui
                    .small_button(icons::X)
                    .on_hover_text("Remove from all selected")
                    .clicked()
                {
                    let targets =
                        selected_entities_with_component(entities, selected, comp.type_id);
                    for entity in targets {
                        actions.push(EditorAction::RemoveComponent {
                            entity,
                            type_id: comp.type_id,
                        });
                    }
                }
            })
            .body(|ui| {
                if let Some(fields) = &comp.fields {
                    if fields.is_empty() {
                        ui.weak("(no fields)");
                    } else {
                        let targets = selected_entities_with_component(
                            entities,
                            selected,
                            comp.type_id,
                        );
                        draw_multi_reflected_fields(
                            ui,
                            comp.type_id,
                            fields,
                            comp.field_metas,
                            &targets,
                            is_read_only,
                            actions,
                        );
                    }
                } else {
                    ui.weak("(no reflection)");
                }
            });
        }
    });
}

/// Renders merged fields for multi-entity editing.
fn draw_multi_reflected_fields(
    ui: &mut egui::Ui,
    type_id: TypeId,
    fields: &[(String, MultiFieldValue)],
    field_metas: Option<&'static [FieldMeta]>,
    targets: &[Entity],
    read_only: bool,
    actions: &mut Vec<EditorAction>,
) {
    egui::Grid::new(format!("multi_fields_{:?}", type_id))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for (name, multi_val) in fields {
                let choices = choices_for(field_metas, name);
                match multi_val {
                    MultiFieldValue::Uniform(value) => {
                        ui.label(name);
                        if read_only {
                            draw_readonly_value(ui, value, choices);
                        } else if let Some(new_value) =
                            draw_value_widget(ui, value, name, choices)
                        {
                            for &entity in targets {
                                actions.push(EditorAction::SetField {
                                    entity,
                                    type_id,
                                    field: name.clone(),
                                    value: new_value.clone(),
                                });
                            }
                        }
                    }
                    MultiFieldValue::Mixed(base) => {
                        ui.label(format!("{name} \u{2014}"));
                        if read_only {
                            draw_readonly_value(ui, base, choices);
                        } else if let Some(new_value) =
                            draw_value_widget(ui, base, name, choices)
                        {
                            for &entity in targets {
                                actions.push(EditorAction::SetField {
                                    entity,
                                    type_id,
                                    field: name.clone(),
                                    value: new_value.clone(),
                                });
                            }
                        }
                    }
                }
                ui.end_row();
            }
        });
}
