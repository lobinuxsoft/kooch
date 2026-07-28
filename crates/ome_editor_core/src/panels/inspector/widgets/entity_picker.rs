//! Entity-reference picker for `ReflectValue::EntityRef` fields.
//!
//! The field was read-only until #655: a `Joint` could be added and never
//! told which two bodies it held, which made it a component that could not
//! do anything. Two gestures assign one now — picking from the dropdown
//! and dropping an entity from the World panel — and both write the same
//! `EntityRef` that code writes, so none of the three is a special case.
//!
//! # Why the name and not the handle
//!
//! `4:1` is not an answer to "which body is this". The label shows the
//! target's `Name`, falling back to the handle only for an entity that has
//! none — which is also how the World panel labels its rows, so the same
//! entity reads the same way in both places.

use ome_ecs::entity::Entity;
use ome_ecs::reflect::{EntityRef, ReflectValue};

use crate::panels::world::entity_row::display_name_for;
use crate::state::EntityDisplayInfo;

/// Renders the picker for a `ReflectValue::EntityRef` field. Returns
/// `Some(new_value)` when the user picks a different target or clears it.
///
/// `requires` is the short name of a component the target must carry, or
/// empty when anything will do. A `Joint` body without a `RigidBody` is
/// not a body: accepting it would leave the joint silently inert, which is
/// indistinguishable from the joint being broken.
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
    let search_id = ui.id().with(("entity_picker_search", salt));

    let combo = egui::ComboBox::from_id_salt(("entity_picker", salt))
        .selected_text(selected_text)
        .show_ui(ui, |ui| {
            // Everything below runs only while the popup is open. In an
            // immediate-mode UI the cost of a panel's draw is paid every
            // frame it is visible, and filtering a scene's entities is not
            // something to pay for a closed dropdown.
            let mut query: String = ui
                .ctx()
                .data(|d| d.get_temp::<String>(search_id))
                .unwrap_or_default();
            let search = ui.add(
                egui::TextEdit::singleline(&mut query)
                    .desired_width(f32::INFINITY)
                    .hint_text("\u{1f50d} Search…"),
            );
            if search.changed() {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(search_id, query.clone()));
            }

            ui.separator();

            if ui.selectable_label(current.is_none(), "(None)").clicked() && current.is_some() {
                new_value = Some(ReflectValue::EntityRef(None));
            }

            let needle = query.trim().to_lowercase();
            let mut shown = 0usize;
            for info in entities.iter().filter(|i| accepts(i, requires)) {
                let label = label_for(info);
                if !needle.is_empty() && !label.to_lowercase().contains(&needle) {
                    continue;
                }
                shown += 1;
                let selected = current_entity == Some(info.entity);
                let resp = ui
                    .selectable_label(selected, label)
                    .on_hover_text(handle_of(info.entity));
                if !selected && resp.clicked() {
                    new_value = Some(assign(info.entity));
                }
            }
            if shown == 0 {
                match requires.is_empty() {
                    true => ui.weak("(no entities)"),
                    false => ui.weak(format!("(no entity carries a {requires})")),
                };
            }
        });

    // Drop target: an entity dragged out of the World panel, which sets a
    // bare `Entity` as its payload — the same one reparenting uses.
    //
    // `dnd_release_payload` takes the payload before checking anything, so
    // the refusal has to happen while hovering, not after the drop.
    let slot = combo.response;
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
                egui::show_tooltip_text(ui.ctx(), ui.layer_id(), slot.id.with("refused"), reason);
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
mod tests {
    use super::*;
    use crate::state::ComponentDisplayInfo;

    fn component(name: &str, fields: Vec<(String, ReflectValue)>) -> ComponentDisplayInfo {
        ComponentDisplayInfo {
            type_id: std::any::TypeId::of::<()>(),
            component: ome_ecs::ComponentId(0),
            short_name: name.to_owned(),
            fields: Some(fields),
            field_metas: None,
            visibility: Default::default(),
        }
    }

    fn entity(index: u32, components: Vec<ComponentDisplayInfo>) -> EntityDisplayInfo {
        EntityDisplayInfo {
            entity: Entity::new(index, 0),
            components,
            parent: None,
            children: Vec::new(),
            depth: 0,
            global_rotation: None,
            scene: None,
            parent_global_rotation: None,
        }
    }

    fn named(index: u32, name: &str) -> EntityDisplayInfo {
        entity(
            index,
            vec![component(
                "Name",
                vec![("value".into(), ReflectValue::String(name.to_owned()))],
            )],
        )
    }

    /// `4:1` is not an answer to "which body is this".
    #[test]
    fn an_entity_reads_as_its_name() {
        assert_eq!(label_for(&named(4, "Door frame")), "Door frame");
    }

    /// An entity with no name still has to be pickable, and the handle is
    /// the only thing left to call it.
    #[test]
    fn a_nameless_entity_falls_back_to_its_handle() {
        assert_eq!(label_for(&entity(7, Vec::new())), "Entity 7:0");
    }

    /// A joint body without a rigid body is not a body. Accepting it would
    /// leave the joint inert, which looks exactly like it being broken.
    #[test]
    fn a_requirement_excludes_what_does_not_carry_it() {
        let plain = named(1, "Marker");
        let body = entity(
            2,
            vec![
                component("RigidBody", Vec::new()),
                component("Name", vec![]),
            ],
        );

        assert!(!accepts(&plain, "RigidBody"));
        assert!(accepts(&body, "RigidBody"));
    }

    /// A field with no requirement takes anything — most references have
    /// nothing in particular to demand.
    #[test]
    fn no_requirement_accepts_anything() {
        assert!(accepts(&named(1, "Marker"), ""));
    }
}
