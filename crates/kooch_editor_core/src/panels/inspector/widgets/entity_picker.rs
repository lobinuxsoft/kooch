//! Entity-reference picker for `ReflectValue::EntityRef` fields.

use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::{EntityRef, ReflectValue};

use crate::panels::world::entity_row::display_name_for;
use crate::state::EntityDisplayInfo;

/// Renders the picker for a `ReflectValue::EntityRef` field. Returns `Some(new_value)` when the
/// user picks a different target or clears it.
pub(crate) fn draw_entity_picker(
    ui: &mut egui::Ui,
    current: Option<EntityRef>,
    entities: &[EntityDisplayInfo],
    requires: &str,
    salt: &str,
) -> Option<ReflectValue> {
    let current_entity = current.and_then(EntityRef::entity);
    let current_info = current_entity.and_then(|e| entities.iter().find(|i| i.entity == e));

    let selected_text = match (current, current_info) {
        (None, _) => "(None)".to_owned(),
        (Some(_), Some(info)) => label_for(info),
        // Named but not present: either the target is gone, or the
        // reference is still persistent because the scene holding it is
        // not open. Those are different situations and read differently.
        (Some(reference), None) => match reference.is_unresolved() {
            true => format!("(not loaded: {reference})"),
            false => format!("(missing: {reference})"),
        },
    };

    let mut new_value: Option<ReflectValue> = None;

    // The list runs only while the popup is open: filtering a scene's entities is not something to
    // pay for a closed dropdown every frame.
    let slot = super::search_combo::search_combo(
        ui,
        ("entity_picker", salt),
        selected_text,
        |ui, needle| {
            if ui.selectable_label(current.is_none(), "(None)").clicked() {
                if current.is_some() {
                    new_value = Some(ReflectValue::EntityRef(None));
                }
                ui.close();
            }

            let mut shown = 0usize;
            for info in entities.iter().filter(|i| accepts(i, requires)) {
                let label = label_for(info);
                if !needle.is_empty() && !label.to_lowercase().contains(needle) {
                    continue;
                }
                shown += 1;
                let selected = current_entity == Some(info.entity);
                let resp = ui
                    .selectable_label(selected, label)
                    .on_hover_text(handle_of(info.entity));
                if resp.clicked() {
                    if !selected {
                        new_value = Some(assign(info.entity));
                    }
                    ui.close();
                }
            }
            if shown == 0 {
                match requires.is_empty() {
                    true => ui.weak("(no entities)"),
                    false => ui.weak(format!("(no entity carries a {requires})")),
                };
            }
        },
    );

    // Drop target: an entity dragged out of the World panel, which sets a bare `Entity` as its
    // payload — the same one reparenting uses.
    if let Some(hovered) = slot.dnd_hover_payload::<Entity>() {
        let dropped = *hovered;
        let info = entities.iter().find(|i| i.entity == dropped);
        match info.filter(|i| accepts(i, requires)) {
            Some(_) => {
                ui.painter().rect_filled(
                    slot.rect,
                    2.0,
                    egui::Color32::from_rgba_unmultiplied(60, 200, 100, 40),
                );
                if slot.dnd_release_payload::<Entity>().is_some() && current_entity != Some(dropped)
                {
                    new_value = Some(assign(dropped));
                }
            }
            None => {
                // Refused, and said so: an accepted-then-inert reference
                // is the failure this whole issue was about.
                ui.painter().rect_filled(
                    slot.rect,
                    2.0,
                    egui::Color32::from_rgba_unmultiplied(200, 80, 80, 40),
                );
                let reason = match info {
                    Some(i) => format!(
                        "{} carries no {requires}, so it cannot be used here",
                        label_for(i),
                    ),
                    None => "that entity is not in the loaded scenes".to_owned(),
                };
                egui::Tooltip::always_open(
                    ui.ctx().clone(),
                    ui.layer_id(),
                    slot.id.with("refused"),
                    egui::PopupAnchor::Pointer,
                )
                .show(|ui| ui.label(reason));
                // Take the payload so the drop ends here rather than
                // falling through to whatever is underneath.
                let _ = slot.dnd_release_payload::<Entity>();
            }
        }
    }

    new_value
}

/// The value a pick writes: a live reference, the same thing code assigns.
fn assign(entity: Entity) -> ReflectValue {
    ReflectValue::EntityRef(Some(EntityRef::live(entity)))
}

/// Whether `info` may be the target of a field requiring `requires`.
fn accepts(info: &EntityDisplayInfo, requires: &str) -> bool {
    requires.is_empty() || info.components.iter().any(|c| c.short_name == requires)
}

/// How an entity reads in the list: its name, or its handle when it has
/// none. Matches the World panel, so the same entity looks the same in
/// both.
fn label_for(info: &EntityDisplayInfo) -> String {
    display_name_for(info).unwrap_or_else(|| format!("Entity {}", handle_of(info.entity)))
}

fn handle_of(entity: Entity) -> String {
    format!("{}:{}", entity.index(), entity.generation())
}

#[cfg(test)]
mod tests;
