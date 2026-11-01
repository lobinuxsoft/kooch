//! The selected element's attributes, its text and its inline style. Each edit goes to the markup in
//! memory; Save writes the file.

use crate::ui_builder::UiBuilder;

pub(super) fn draw(ui: &mut egui::Ui, builder: &mut UiBuilder) {
    ui.strong("Inspector");
    ui.separator();
    let Some(origin) = builder.selected else {
        ui.weak("Select an element in the hierarchy or on the canvas.");
        return;
    };
    let Some(inspected) = builder.inspect(origin) else {
        builder.selected = None;
        return;
    };
    egui::ScrollArea::vertical()
        .id_salt("ui_builder_inspector")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.label(egui::RichText::new(format!("<{}>", inspected.tag)).monospace());
            egui::Grid::new("ui_builder_fields")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    if let Some(value) = field(ui, "Name", &inspected.name) {
                        builder.set_attribute(origin, "name", &value);
                    }
                    if let Some(value) = field(ui, "Class", &inspected.classes) {
                        builder.set_attribute(origin, "class", &value);
                    }
                    if let Some(text) = &inspected.text
                        && let Some(value) = field(ui, "Text", text)
                    {
                        builder.set_text(origin, &value);
                    }
                    for (key, value) in &inspected.attributes {
                        if let Some(value) = field(ui, key, value) {
                            builder.set_attribute(origin, key, &value);
                        }
                    }
                });
            add_attribute(ui, builder, origin);
            ui.separator();
            ui.strong("Inline style").on_hover_text(
                "CSS declarations, one per line. They beat any rule of the sheet, as `style` does \
                 in CSS.",
            );
            inline_style(ui, builder, origin, &inspected.style);
        });
}

/// A labelled one-line field; the new value when it changed.
fn field(ui: &mut egui::Ui, label: &str, value: &str) -> Option<String> {
    ui.label(label);
    let mut value = value.to_owned();
    let changed = ui
        .add(egui::TextEdit::singleline(&mut value).desired_width(f32::INFINITY))
        .changed();
    ui.end_row();
    changed.then_some(value)
}

fn add_attribute(ui: &mut egui::Ui, builder: &mut UiBuilder, origin: u32) {
    let id = ui.make_persistent_id("ui_builder_new_attribute");
    let (mut key, mut value) = ui
        .data(|data| data.get_temp::<(String, String)>(id))
        .unwrap_or_default();
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut key)
                .hint_text("attribute")
                .desired_width(90.0),
        );
        ui.add(
            egui::TextEdit::singleline(&mut value)
                .hint_text("value")
                .desired_width(90.0),
        );
        if ui
            .add_enabled(!key.trim().is_empty(), egui::Button::new("Add"))
            .on_hover_text("`on:click`, `bind:value`, `min`… as the file writes them")
            .clicked()
        {
            builder.set_attribute(origin, key.trim(), &value);
            key.clear();
            value.clear();
        }
    });
    ui.data_mut(|data| data.insert_temp(id, (key, value)));
}

/// One declaration per line while it is edited; `a: b; c: d` in the file.
fn inline_style(ui: &mut egui::Ui, builder: &mut UiBuilder, origin: u32, style: &str) {
    // Kept while focused: re-reading the attribute would eat the empty line just typed.
    let id = ui.make_persistent_id(("ui_builder_style", origin));
    let mut text = ui
        .data(|data| data.get_temp::<String>(id))
        .unwrap_or_else(|| declarations(style).join(";\n"));
    let response = ui.add(
        egui::TextEdit::multiline(&mut text)
            .code_editor()
            .desired_rows(4)
            .desired_width(f32::INFINITY)
            .hint_text("background-color: #14171f;\nwidth: 240px"),
    );
    if response.changed() {
        builder.set_attribute(origin, "style", &declarations(&text).join("; "));
    }
    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(id, text));
    } else {
        ui.data_mut(|data| data.remove::<String>(id));
    }
}

fn declarations(style: &str) -> Vec<&str> {
    style
        .split([';', '\n'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}
