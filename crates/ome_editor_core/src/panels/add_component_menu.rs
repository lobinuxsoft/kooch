//! Shared helper for the "Add Component" menu.
//!
//! Groups reflected component types by their `category` attribute:
//! uncategorized entries render at the top level, and each category
//! gets its own submenu.

use ome_ecs::component::ComponentId;
use std::collections::BTreeMap;

use crate::state::ReflectedTypeInfo;

/// Draws a categorized list of reflected component types inside `ui`.
///
/// Each selectable entry invokes `on_select` with the chosen
/// [`ComponentId`]. Uncategorized types (those without
/// `#[reflect(category = "...")]`) render as flat entries before the
/// category submenus.
pub(crate) fn draw_categorized(
    ui: &mut egui::Ui,
    available: &[&ReflectedTypeInfo],
    mut on_select: impl FnMut(ComponentId),
) {
    let mut uncategorized: Vec<&ReflectedTypeInfo> = Vec::new();
    let mut by_category: BTreeMap<&str, Vec<&ReflectedTypeInfo>> = BTreeMap::new();
    for type_info in available {
        match type_info.category.as_deref() {
            Some(cat) => by_category.entry(cat).or_default().push(type_info),
            None => uncategorized.push(type_info),
        }
    }

    // Each entry keyed on the component it adds, not on where it landed
    // in the list.
    //
    // `available` is what the entity does *not* already carry, so the list
    // changes with the selection: an entry taking an automatic id — handed
    // out by order of creation — is renamed by every component added or
    // removed above it, while the menu stays open in the same place. Same
    // rect, new id, which is what egui reports (#641).
    for type_info in &uncategorized {
        let clicked = ui
            .push_id(type_info.component, |ui| {
                ui.selectable_label(false, &type_info.short_name).clicked()
            })
            .inner;
        if clicked {
            on_select(type_info.component);
            ui.close();
        }
    }

    if !uncategorized.is_empty() && !by_category.is_empty() {
        ui.separator();
    }

    for (category, entries) in &by_category {
        ui.push_id(*category, |ui| {
            ui.menu_button(*category, |ui| {
                for type_info in entries {
                    let clicked = ui
                        .push_id(type_info.component, |ui| {
                            ui.selectable_label(false, &type_info.short_name).clicked()
                        })
                        .inner;
                    if clicked {
                        on_select(type_info.component);
                        ui.close();
                    }
                }
            });
        });
    }
}
