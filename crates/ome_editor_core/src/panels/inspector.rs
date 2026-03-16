//! Inspector panel — component details for selected entities.

use std::any::TypeId;
use std::collections::HashSet;

use ome_ecs::entity::Entity;
use ome_ecs::reflect::ReflectValue;

use crate::actions::EditorAction;
use crate::icons;
use crate::state::{EntityDisplayInfo, ReflectedTypeInfo};

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
        ui.label(format!("{} entities selected", selected.len()));
        ui.separator();
        for &entity in selected {
            ui.label(format!(
                "{} Entity {}:{}",
                icons::CUBE,
                entity.index(),
                entity.generation()
            ));
        }
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

    // "Add Component" dropdown.
    let existing: HashSet<TypeId> = info.components.iter().map(|c| c.type_id).collect();
    let available: Vec<&ReflectedTypeInfo> = reflected_types
        .iter()
        .filter(|t| !existing.contains(&t.type_id))
        .collect();

    if !available.is_empty() {
        egui::ComboBox::from_label(format!("{} Add Component", icons::PLUS))
            .selected_text("Select...")
            .show_ui(ui, |ui| {
                for type_info in &available {
                    if ui.selectable_label(false, &type_info.short_name).clicked() {
                        actions.push(EditorAction::AddComponent {
                            entity,
                            type_id: type_info.type_id,
                        });
                    }
                }
            });
        ui.separator();
    }

    if info.components.is_empty() {
        ui.weak("(no components)");
        return;
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        for comp in &info.components {
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
                if ui
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
                if let Some(new_value) = draw_value_widget(ui, value) {
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

/// Draws an editable widget for a single reflected value.
/// Returns `Some(new_value)` if the user modified it.
fn draw_value_widget(ui: &mut egui::Ui, value: &ReflectValue) -> Option<ReflectValue> {
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
        ReflectValue::Vec4(v) => {
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
                    .add(egui::DragValue::new(&mut dx).speed(0.5).suffix("°"))
                    .changed();
                ui.label("y");
                changed |= ui
                    .add(egui::DragValue::new(&mut dy).speed(0.5).suffix("°"))
                    .changed();
                ui.label("z");
                changed |= ui
                    .add(egui::DragValue::new(&mut dz).speed(0.5).suffix("°"))
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
