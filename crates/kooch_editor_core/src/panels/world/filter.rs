//! The World panel's filter: narrow two thousand rows to the ones being
//! looked for.
//!
//! # 🔴 By COMPONENT, not only by name
//!
//! A name box answers *"where is the thing I called Sun"*. It cannot
//! answer *"how many directional lights are in this scene"* — and that
//! is the question that cost a day and a half of shadow debugging, with
//! the answer sitting in row 2156 of a list nobody can read (#1001).
//!
//! The type menu carries a COUNT beside every name for the same reason.
//! `DirectionalLight (2)` states the fault without anyone having to
//! filter by it: the number is the finding, the filter is the follow-up.

use std::collections::BTreeMap;

use crate::icons;
use crate::state::EntityDisplayInfo;

use super::entity_row::display_name_for;

/// What the panel is currently narrowed to.
///
/// 🔴 Kept in egui's TEMP store, not the persisted one — deliberately
/// unlike the group open/closed flags next to it. A collapsed group is
/// visible as a collapsed group; a filter is invisible except by the
/// rows it removed, so one that survived a restart would be a panel
/// quietly lying about what the scene contains.
#[derive(Clone, Default)]
pub(super) struct WorldFilter {
    /// Case-insensitive substring of the entity's name.
    pub(super) text: String,
    /// Short type name every listed entity must carry.
    pub(super) component: Option<String>,
}

impl WorldFilter {
    /// Whether anything is being hidden. Everything downstream branches
    /// on this rather than on the fields, so "no filter" is one idea
    /// with one definition.
    pub(super) fn active(&self) -> bool {
        !self.text.trim().is_empty() || self.component.is_some()
    }

    /// Whether this entity survives the filter. Both terms are ANDed:
    /// two narrowings that widened each other would be a filter nobody
    /// could predict.
    pub(super) fn matches(&self, info: &EntityDisplayInfo) -> bool {
        let by_text = match self.text.trim() {
            "" => true,
            needle => {
                let needle = needle.to_lowercase();
                // The name if it has one, and the handle if it does not —
                // an unnamed entity is still findable by what the row
                // actually shows.
                display_name_for(info)
                    .unwrap_or_else(|| info.entity.to_string())
                    .to_lowercase()
                    .contains(&needle)
            }
        };
        let by_component = match &self.component {
            None => true,
            Some(wanted) => info.components.iter().any(|c| c.short_name == *wanted),
        };
        by_text && by_component
    }
}

/// Every component type present in the world, with how many entities
/// carry it.
///
/// Built from the ENTITIES rather than from the type registry: a menu of
/// everything registered is a menu of two hundred types the scene does
/// not contain, and the count — the half that answers a question on its
/// own — only exists for what is really there.
fn present_types(entities: &[EntityDisplayInfo]) -> BTreeMap<&str, usize> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for info in entities {
        for component in &info.components {
            *counts.entry(component.short_name.as_ref()).or_insert(0) += 1;
        }
    }
    counts
}

/// Draws the filter row and reports what it now holds.
pub(super) fn draw_filter_bar(
    ui: &mut egui::Ui,
    entities: &[EntityDisplayInfo],
    filter: &mut WorldFilter,
) {
    ui.horizontal(|ui| {
        ui.label(icons::MAGNIFYING_GLASS);
        // Takes what is left after the type button and the clear cross,
        // measured rather than guessed: a fixed width leaves a gap in a
        // wide panel and overflows a narrow one.
        let reserved = 132.0;
        let width = (ui.available_width() - reserved).max(48.0);
        ui.add(
            egui::TextEdit::singleline(&mut filter.text)
                .desired_width(width)
                .hint_text("Filter by name"),
        );

        let counts = present_types(entities);
        let label = match &filter.component {
            Some(name) => name.clone(),
            None => "Any type".to_owned(),
        };
        ui.menu_button(label, |ui| {
            if ui.button("Any type").clicked() {
                filter.component = None;
                ui.close();
            }
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    for (name, count) in &counts {
                        // The count is the point. See the module note.
                        if ui.button(format!("{name}  ({count})")).clicked() {
                            filter.component = Some((*name).to_owned());
                            ui.close();
                        }
                    }
                });
        });

        // Only when there is something to clear, so the row does not
        // carry a permanently dead button.
        if filter.active()
            && ui
                .button(icons::X)
                .on_hover_text("Clear the filter")
                .clicked()
        {
            *filter = WorldFilter::default();
        }
    });
}
