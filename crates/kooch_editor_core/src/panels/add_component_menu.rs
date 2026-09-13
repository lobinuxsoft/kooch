//! Shared helper for the "Add Component" menu.

use kooch_ecs::component::ComponentId;
use std::collections::BTreeMap;

use crate::state::ReflectedTypeInfo;

/// Draws a categorized list of reflected component types inside `ui`.
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

    // Each entry keyed on the component it adds, not on where it landed in the list.
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
