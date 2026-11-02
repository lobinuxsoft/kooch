//! An `on:*` field: free text, a ▾ with every action the project already uses, and a mark for
//! whether a system reads it. A typo shows as an action nothing reads.

use kooch_ui::schema::AttributeSpec;

use super::inspector::{label, unset};
use crate::icons;
use crate::panels::inspector::AssetCatalogEntry;
use crate::ui_builder::UiBuilder;

/// Room the ▾, the mark and the copy button take beside the text.
const TRAILING: f32 = 72.0;

pub(super) fn row(
    ui: &mut egui::Ui,
    builder: &mut UiBuilder,
    origin: u32,
    spec: &AttributeSpec,
    value: Option<String>,
    catalog: &[AssetCatalogEntry],
) {
    label(ui, spec.key, spec.hint, value.is_some());
    let known: Vec<(String, bool, bool)> = builder
        .actions(catalog)
        .iter()
        .map(|action| (action.name.clone(), action.sent, action.read))
        .collect();
    let mut picked = None;
    ui.horizontal(|ui| {
        let mut text = value.clone().unwrap_or_default();
        let edit = egui::TextEdit::singleline(&mut text)
            .hint_text("menu.resume")
            .desired_width((ui.available_width() - TRAILING).max(40.0));
        if ui.add(edit).changed() {
            picked = Some(text);
        }
        crate::widgets::search_menu(
            ui,
            ("ui_builder_action", origin, spec.key),
            Some(14.0),
            String::new(),
            |ui, needle| {
                let mut shown = 0;
                for (name, sent, read) in &known {
                    if !needle.is_empty() && !name.to_lowercase().contains(needle) {
                        continue;
                    }
                    let mark = if *read { icons::CHECK } else { icons::WARNING };
                    let response = ui
                        .selectable_label(value.as_ref() == Some(name), format!("{mark} {name}"))
                        .on_hover_text(why(*sent, *read));
                    if response.clicked() {
                        picked = Some(name.clone());
                        ui.close();
                    }
                    shown += 1;
                }
                if shown == 0 {
                    ui.weak("(none yet: type a name)");
                }
            },
        );
        let Some(name) = value.as_deref().filter(|name| !name.is_empty()) else {
            return;
        };
        let read = known.iter().any(|(own, _, read)| own == name && *read);
        if read {
            ui.label(icons::CHECK)
                .on_hover_text("A system reads it with ui_actions");
        } else {
            ui.colored_label(ui.visuals().warn_fg_color, icons::WARNING)
                .on_hover_text("No system of the project reads it: a typo, or code still to write");
        }
        if ui
            .small_button(icons::COPY)
            .on_hover_text("Copy the line a system reads it with")
            .clicked()
        {
            ui.ctx()
                .copy_text(format!("ui_actions(resources, \"{name}\")"));
        }
    });
    unset(ui, builder, origin, spec.key, value.is_some());
    ui.end_row();
    if let Some(name) = picked {
        builder.set_attribute(origin, spec.key, name.trim());
    }
}

fn why(sent: bool, read: bool) -> &'static str {
    match (sent, read) {
        (true, true) => "Sent by a .kxml and read by a system",
        (true, false) => "Sent by a .kxml, but no system reads it",
        (false, _) => "Read by a system, but no .kxml sends it yet",
    }
}
