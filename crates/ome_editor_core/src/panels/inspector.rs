//! Inspector panel — component details for selected entities.

use std::any::TypeId;
use std::collections::{HashMap, HashSet};

use ome_ecs::entity::Entity;
use ome_ecs::reflect::{InspectorVisibility, ReflectValue};

use crate::actions::EditorAction;
use crate::icons;
use crate::state::{ComponentDisplayInfo, EntityDisplayInfo, ReflectedTypeInfo};

// ---------------------------------------------------------------------------
// Multi-entity types
// ---------------------------------------------------------------------------

/// A field value across multiple selected entities.
enum MultiFieldValue {
    /// All selected entities share the same value.
    Uniform(ReflectValue),
    /// Values differ — stores the first entity's value as DragValue base.
    Mixed(ReflectValue),
}

/// Merged component info across multiple selected entities.
struct MultiComponentInfo {
    type_id: TypeId,
    short_name: String,
    /// How many of the selected entities have this component.
    present_count: usize,
    /// Total number of selected entities.
    total_count: usize,
    /// Merged fields (`None` = no reflection).
    fields: Option<Vec<(String, MultiFieldValue)>>,
    visibility: InspectorVisibility,
}

// ---------------------------------------------------------------------------
// Multi-entity data gathering
// ---------------------------------------------------------------------------

/// Collects shared component data across `selected` entities.
fn gather_multi_component_info(
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
    }
    let mut comp_map: HashMap<TypeId, CompMeta> = HashMap::new();

    for info in &selected_infos {
        for comp in &info.components {
            let entry = comp_map.entry(comp.type_id).or_insert(CompMeta {
                short_name: comp.short_name.clone(),
                count: 0,
                visibility: comp.visibility,
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
            visibility: meta.visibility,
        });
    }

    result.sort_by(|a, b| a.short_name.cmp(&b.short_name));
    result
}

/// Returns the subset of `selected` entities that have a component with `type_id`.
fn selected_entities_with_component(
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

// ---------------------------------------------------------------------------
// Public draw function
// ---------------------------------------------------------------------------

/// Content of the "Inspector" tab — component details for selected entities.
pub(crate) fn draw_inspector_content(
    ui: &mut egui::Ui,
    entities: &[EntityDisplayInfo],
    selected: &[Entity],
    reflected_types: &[ReflectedTypeInfo],
    actions: &mut Vec<EditorAction>,
) {
    if selected.is_empty() {
        ui.weak("No entity selected");
        return;
    }

    if selected.len() > 1 {
        draw_multi_entity_inspector(ui, entities, selected, reflected_types, actions);
        return;
    }

    // Single entity selected — show full inspector.
    let entity = selected[0];
    let Some(info) = entities.iter().find(|e| e.entity == entity) else {
        ui.weak("Entity not found (despawned?)");
        return;
    };

    let entity_name = info
        .components
        .iter()
        .find(|c| c.short_name == "Name")
        .and_then(|c| c.fields.as_ref())
        .and_then(|fields| {
            fields.iter().find_map(|(name, val)| {
                if name == "value" {
                    if let ReflectValue::String(s) = val {
                        if !s.is_empty() {
                            return Some(s.clone());
                        }
                    }
                }
                None
            })
        });

    if let Some(name) = &entity_name {
        ui.label(format!(
            "{} {}  ({}:{})",
            icons::CUBE,
            name,
            entity.index(),
            entity.generation()
        ));
    } else {
        ui.label(format!(
            "{} Entity  index: {}  generation: {}",
            icons::CUBE,
            entity.index(),
            entity.generation()
        ));
    }
    ui.separator();

    // Editable name field (separate from component list).
    draw_name_editor(ui, entity, info, actions);

    // "Add Component" dropdown.
    let existing: HashSet<TypeId> = info.components.iter().map(|c| c.type_id).collect();
    let available: Vec<&ReflectedTypeInfo> = reflected_types
        .iter()
        .filter(|t| !existing.contains(&t.type_id))
        .collect();

    if !available.is_empty() {
        ui.menu_button(format!("{} Add Component", icons::PLUS), |ui| {
            crate::panels::add_component_menu::draw_categorized(ui, &available, |type_id| {
                actions.push(EditorAction::AddComponent { entity, type_id });
            });
        });
        ui.separator();
    }

    // Filter out Hidden components for display.
    let visible_components: Vec<&ComponentDisplayInfo> = info
        .components
        .iter()
        .filter(|c| c.visibility != InspectorVisibility::Hidden)
        .collect();

    if visible_components.is_empty() {
        ui.weak("(no components)");
        return;
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        for comp in &visible_components {
            let is_read_only = comp.visibility == InspectorVisibility::ReadOnly;
            let id = ui.make_persistent_id(format!(
                "comp_{}_{:?}",
                entity.index(),
                comp.type_id
            ));
            egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                true,
            )
            .show_header(ui, |ui| {
                ui.strong(format!("{} {}", icons::PUZZLE_PIECE, &comp.short_name));
                if !is_read_only
                    && ui
                        .small_button(icons::X)
                        .on_hover_text("Remove component")
                        .clicked()
                {
                    actions.push(EditorAction::RemoveComponent {
                        entity,
                        type_id: comp.type_id,
                    });
                }
            })
            .body(|ui| {
                if let Some(fields) = &comp.fields {
                    if fields.is_empty() {
                        ui.weak("(no fields)");
                    } else if is_read_only {
                        draw_readonly_fields(ui, entity, comp.type_id, fields);
                    } else {
                        draw_reflected_fields(ui, entity, comp.type_id, fields, actions);
                    }
                } else {
                    ui.weak("(no reflection)");
                }
            });
        }
    });
}

// ---------------------------------------------------------------------------
// Single-entity name editor
// ---------------------------------------------------------------------------

/// Draws an editable name field for the Name component (shown above the component list).
fn draw_name_editor(
    ui: &mut egui::Ui,
    entity: Entity,
    info: &EntityDisplayInfo,
    actions: &mut Vec<EditorAction>,
) {
    let name_comp = info.components.iter().find(|c| c.short_name == "Name");
    let Some(comp) = name_comp else { return };
    let Some(fields) = &comp.fields else { return };
    let Some((_, value)) = fields.iter().find(|(n, _)| n == "value") else {
        return;
    };
    let ReflectValue::String(current) = value else {
        return;
    };

    let mut val = current.clone();
    ui.horizontal(|ui| {
        ui.label("Name");
        if ui.text_edit_singleline(&mut val).changed() {
            actions.push(EditorAction::SetField {
                entity,
                type_id: comp.type_id,
                field: "value".to_owned(),
                value: ReflectValue::String(val),
            });
        }
    });
    ui.separator();
}

// ---------------------------------------------------------------------------
// Multi-entity inspector
// ---------------------------------------------------------------------------

/// Draws the multi-entity inspector when more than one entity is selected.
fn draw_multi_entity_inspector(
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

                if !is_read_only
                    && ui
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

// ---------------------------------------------------------------------------
// Multi-entity field rendering
// ---------------------------------------------------------------------------

/// Renders merged fields for multi-entity editing.
fn draw_multi_reflected_fields(
    ui: &mut egui::Ui,
    type_id: TypeId,
    fields: &[(String, MultiFieldValue)],
    targets: &[Entity],
    read_only: bool,
    actions: &mut Vec<EditorAction>,
) {
    egui::Grid::new(format!("multi_fields_{:?}", type_id))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for (name, multi_val) in fields {
                match multi_val {
                    MultiFieldValue::Uniform(value) => {
                        ui.label(name);
                        if read_only {
                            draw_readonly_value(ui, value);
                        } else if let Some(new_value) = draw_value_widget(ui, value, name) {
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
                            draw_readonly_value(ui, base);
                        } else if let Some(new_value) = draw_value_widget(ui, base, name) {
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

// ---------------------------------------------------------------------------
// Single-entity field rendering
// ---------------------------------------------------------------------------

/// Renders editable widgets for reflected component fields.
fn draw_reflected_fields(
    ui: &mut egui::Ui,
    entity: Entity,
    type_id: TypeId,
    fields: &[(String, ReflectValue)],
    actions: &mut Vec<EditorAction>,
) {
    egui::Grid::new(format!("fields_{:?}_{}", type_id, entity.index()))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for (name, value) in fields {
                ui.label(name);
                if let Some(new_value) = draw_value_widget(ui, value, name) {
                    actions.push(EditorAction::SetField {
                        entity,
                        type_id,
                        field: name.clone(),
                        value: new_value,
                    });
                }
                ui.end_row();
            }
        });
}

/// Renders read-only display for component fields.
fn draw_readonly_fields(
    ui: &mut egui::Ui,
    entity: Entity,
    type_id: TypeId,
    fields: &[(String, ReflectValue)],
) {
    egui::Grid::new(format!("ro_fields_{:?}_{}", type_id, entity.index()))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for (name, value) in fields {
                ui.label(name);
                draw_readonly_value(ui, value);
                ui.end_row();
            }
        });
}

/// Renders a read-only display for a single value.
fn draw_readonly_value(ui: &mut egui::Ui, value: &ReflectValue) {
    ui.weak(format!("{value}"));
}

// ---------------------------------------------------------------------------
// Value widgets
// ---------------------------------------------------------------------------

/// Draws an editable widget for a single reflected value.
/// Returns `Some(new_value)` if the user modified it.
/// `field_name` is used to detect color fields and show a color picker.
fn draw_value_widget(ui: &mut egui::Ui, value: &ReflectValue, field_name: &str) -> Option<ReflectValue> {
    match value {
        ReflectValue::F32(v) => {
            let mut val = *v;
            let resp = ui.add(egui::DragValue::new(&mut val).speed(0.1));
            resp.changed().then_some(ReflectValue::F32(val))
        }
        ReflectValue::F64(v) => {
            let mut val = *v as f32;
            let resp = ui.add(egui::DragValue::new(&mut val).speed(0.1));
            resp.changed().then_some(ReflectValue::F64(val as f64))
        }
        ReflectValue::U8(v) => {
            let mut val = *v as i64;
            let resp = ui.add(egui::DragValue::new(&mut val).range(0..=u8::MAX as i64));
            resp.changed().then_some(ReflectValue::U8(val as u8))
        }
        ReflectValue::U16(v) => {
            let mut val = *v as i64;
            let resp = ui.add(egui::DragValue::new(&mut val).range(0..=u16::MAX as i64));
            resp.changed().then_some(ReflectValue::U16(val as u16))
        }
        ReflectValue::U32(v) => {
            let mut val = *v as i64;
            let resp = ui.add(egui::DragValue::new(&mut val).range(0..=u32::MAX as i64));
            resp.changed().then_some(ReflectValue::U32(val as u32))
        }
        ReflectValue::U64(v) => {
            let mut val = *v as i64;
            let resp = ui.add(egui::DragValue::new(&mut val));
            resp.changed()
                .then_some(ReflectValue::U64(val.max(0) as u64))
        }
        ReflectValue::I8(v) => {
            let mut val = *v as i64;
            let resp = ui.add(
                egui::DragValue::new(&mut val).range(i8::MIN as i64..=i8::MAX as i64),
            );
            resp.changed().then_some(ReflectValue::I8(val as i8))
        }
        ReflectValue::I16(v) => {
            let mut val = *v as i64;
            let resp = ui.add(
                egui::DragValue::new(&mut val).range(i16::MIN as i64..=i16::MAX as i64),
            );
            resp.changed().then_some(ReflectValue::I16(val as i16))
        }
        ReflectValue::I32(v) => {
            let mut val = *v;
            let resp = ui.add(egui::DragValue::new(&mut val));
            resp.changed().then_some(ReflectValue::I32(val))
        }
        ReflectValue::I64(v) => {
            let mut val = *v;
            let resp = ui.add(egui::DragValue::new(&mut val));
            resp.changed().then_some(ReflectValue::I64(val))
        }
        ReflectValue::Bool(v) => {
            let mut val = *v;
            let resp = ui.checkbox(&mut val, "");
            resp.changed().then_some(ReflectValue::Bool(val))
        }
        ReflectValue::String(v) => {
            let mut val = v.clone();
            let resp = ui.text_edit_singleline(&mut val);
            resp.changed().then_some(ReflectValue::String(val))
        }
        ReflectValue::Vec2(v) => {
            let mut x = v.x;
            let mut y = v.y;
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label("x");
                changed |= ui.add(egui::DragValue::new(&mut x).speed(0.1)).changed();
                ui.label("y");
                changed |= ui.add(egui::DragValue::new(&mut y).speed(0.1)).changed();
            });
            changed.then_some(ReflectValue::Vec2(glam::Vec2::new(x, y)))
        }
        ReflectValue::Vec3(v) => {
            if field_name.contains("color") {
                let mut rgb = [v.x, v.y, v.z];
                let resp = ui.color_edit_button_rgb(&mut rgb);
                resp.changed()
                    .then_some(ReflectValue::Vec3(glam::Vec3::new(rgb[0], rgb[1], rgb[2])))
            } else {
                let mut x = v.x;
                let mut y = v.y;
                let mut z = v.z;
                let mut changed = false;
                ui.horizontal(|ui| {
                    ui.label("x");
                    changed |= ui.add(egui::DragValue::new(&mut x).speed(0.1)).changed();
                    ui.label("y");
                    changed |= ui.add(egui::DragValue::new(&mut y).speed(0.1)).changed();
                    ui.label("z");
                    changed |= ui.add(egui::DragValue::new(&mut z).speed(0.1)).changed();
                });
                changed.then_some(ReflectValue::Vec3(glam::Vec3::new(x, y, z)))
            }
        }
        ReflectValue::Vec4(v) => {
            let is_color = field_name.contains("color");
            if is_color {
                let mut rgba = [v.x, v.y, v.z, v.w];
                let resp = ui.color_edit_button_rgba_unmultiplied(&mut rgba);
                resp.changed().then_some(ReflectValue::Vec4(glam::Vec4::new(
                    rgba[0], rgba[1], rgba[2], rgba[3],
                )))
            } else {
                let mut vals = [v.x, v.y, v.z, v.w];
                let labels = ["x", "y", "z", "w"];
                let mut changed = false;
                ui.horizontal(|ui| {
                    for (i, label) in labels.iter().enumerate() {
                        ui.label(*label);
                        changed |= ui
                            .add(egui::DragValue::new(&mut vals[i]).speed(0.1))
                            .changed();
                    }
                });
                changed.then_some(ReflectValue::Vec4(glam::Vec4::new(
                    vals[0], vals[1], vals[2], vals[3],
                )))
            }
        }
        ReflectValue::Quat(v) => {
            // Display as Euler angles (degrees) for intuitive editing.
            let (rx, ry, rz) = v.to_euler(glam::EulerRot::XYZ);
            let mut dx = rx.to_degrees() + 0.0; // eliminate -0.0
            let mut dy = ry.to_degrees() + 0.0;
            let mut dz = rz.to_degrees() + 0.0;
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label("x");
                changed |= ui
                    .add(egui::DragValue::new(&mut dx).speed(0.5).suffix("\u{00b0}"))
                    .changed();
                ui.label("y");
                changed |= ui
                    .add(egui::DragValue::new(&mut dy).speed(0.5).suffix("\u{00b0}"))
                    .changed();
                ui.label("z");
                changed |= ui
                    .add(egui::DragValue::new(&mut dz).speed(0.5).suffix("\u{00b0}"))
                    .changed();
            });
            changed.then_some(ReflectValue::Quat(glam::Quat::from_euler(
                glam::EulerRot::XYZ,
                dx.to_radians(),
                dy.to_radians(),
                dz.to_radians(),
            )))
        }
        ReflectValue::Mat4(_) => {
            ui.label("[Mat4]");
            None
        }
    }
}
