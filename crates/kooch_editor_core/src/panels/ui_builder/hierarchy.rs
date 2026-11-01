//! The document's elements as the file nests them; a `<use>` is one row, as Unity shows a template.

use crate::ui_builder::UiBuilder;

/// Points per nesting level.
const INDENT: f32 = 12.0;

pub(super) fn draw(ui: &mut egui::Ui, builder: &mut UiBuilder) {
    ui.strong("Hierarchy");
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("ui_builder_hierarchy")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for row in builder.outline() {
                let mut label = format!("<{}>", row.tag);
                if let Some(name) = &row.name {
                    label.push_str(&format!(" #{name}"));
                }
                if let Some(classes) = &row.classes {
                    for class in classes.split_whitespace() {
                        label.push_str(&format!(" .{class}"));
                    }
                }
                ui.horizontal(|ui| {
                    ui.add_space(row.depth as f32 * INDENT);
                    if ui
                        .selectable_label(builder.selected == Some(row.origin), label)
                        .clicked()
                    {
                        builder.selected = Some(row.origin);
                    }
                });
            }
        });
}
