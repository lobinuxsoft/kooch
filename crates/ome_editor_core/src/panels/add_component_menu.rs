//! Shared helper for the "Add Component" menu.
//!
//! Groups reflected component types by their `category` attribute:
//! uncategorized entries render at the top level, and each category
//! gets its own submenu.

use std::any::TypeId;
use std::collections::BTreeMap;

use crate::state::ReflectedTypeInfo;

/// Draws a categorized list of reflected component types inside `ui`.
///
/// Each selectable entry invokes `on_select` with the chosen `TypeId`.
/// Uncategorized types (those without `#[reflect(category = "...")]`)
/// render as flat entries before the category submenus.
pub(crate) fn draw_categorized(
    ui: &mut egui::Ui,
    available: &[&ReflectedTypeInfo],
    mut on_select: impl FnMut(TypeId),
) {
    let mut uncategorized: Vec<&ReflectedTypeInfo> = Vec::new();
    let mut by_category: BTreeMap<&'static str, Vec<&ReflectedTypeInfo>> = BTreeMap::new();
    for type_info in available {
        match type_info.category {
            Some(cat) => by_category.entry(cat).or_default().push(type_info),
            None => uncategorized.push(type_info),
        }
    }

    for type_info in &uncategorized {
        if ui.selectable_label(false, &type_info.short_name).clicked() {
            on_select(type_info.type_id);
            ui.close();
        }
    }

    if !uncategorized.is_empty() && !by_category.is_empty() {
        ui.separator();
    }

    for (category, entries) in &by_category {
        ui.menu_button(*category, |ui| {
            for type_info in entries {
                if ui.selectable_label(false, &type_info.short_name).clicked() {
                    on_select(type_info.type_id);
                    ui.close();
                }
            }
        });
    }
}
