//! The selected element, as Unity's Builder Inspector lays it out: its attributes as typed fields
//! from the element's schema, then the ones it does not know, then the inline style. A set
//! attribute's label is bold and ↺ unsets it; the inline style by category, with the CSS behind.
//! Each edit goes to the markup in memory.

use std::path::{Component, Path, PathBuf};

use kooch_ui::schema::{AttributeKind, AttributeSpec};
use kooch_ui::source::ElementKind;

use crate::panels::inspector::AssetCatalogEntry;
use crate::ui_builder::{SOURCE_TYPE, UiBuilder};

pub(super) fn draw(ui: &mut egui::Ui, builder: &mut UiBuilder, catalog: &[AssetCatalogEntry]) {
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
    let schema = ElementKind::from_tag(&inspected.tag).map_or(&[][..], ElementKind::attributes);
    egui::ScrollArea::vertical()
        .id_salt("ui_builder_inspector")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.label(egui::RichText::new(format!("<{}>", inspected.tag)).monospace());
            egui::CollapsingHeader::new("Attributes")
                .default_open(true)
                .show(ui, |ui| {
                    egui::Grid::new("ui_builder_fields")
                        .num_columns(3)
                        .striped(true)
                        .show(ui, |ui| {
                            let set = |value: &str| (!value.is_empty()).then(|| value.to_owned());
                            text_row(ui, builder, origin, "name", set(&inspected.name), "");
                            super::classes::row(ui, builder, origin, &inspected.classes);
                            if let Some(text) = &inspected.text {
                                label(ui, "text", "The label's content", true);
                                let mut value = text.clone();
                                if ui.add(text_edit(&mut value, "")).changed() {
                                    builder.set_text(origin, &value);
                                }
                                ui.label("");
                                ui.end_row();
                            }
                            for spec in schema {
                                let value = attribute(&inspected.attributes, spec.key);
                                typed_row(ui, builder, origin, spec, value, catalog);
                            }
                        });
                });
            egui::CollapsingHeader::new("Matching selectors")
                .default_open(true)
                .show(ui, |ui| super::classes::matching(ui, builder, origin));
            let others: Vec<(String, String)> = inspected
                .attributes
                .iter()
                .filter(|(key, _)| schema.iter().all(|spec| spec.key != key))
                .cloned()
                .collect();
            egui::CollapsingHeader::new("Other attributes")
                .default_open(!others.is_empty())
                .show(ui, |ui| {
                    egui::Grid::new("ui_builder_others")
                        .num_columns(3)
                        .show(ui, |ui| {
                            for (key, value) in others {
                                text_row(ui, builder, origin, &key, Some(value), "");
                            }
                        });
                    add_attribute(ui, builder, origin);
                });
            egui::CollapsingHeader::new("Inline style")
                .default_open(true)
                .show(ui, |ui| {
                    super::styles::draw(ui, builder, origin, &inspected.style);
                    egui::CollapsingHeader::new("CSS")
                        .id_salt("ui_builder_css")
                        .show(ui, |ui| {
                            ui.weak("One declaration per line. They beat any rule of the sheet.");
                            inline_style(ui, builder, origin, &inspected.style);
                        });
                });
        });
}

fn attribute(attributes: &[(String, String)], key: &str) -> Option<String> {
    attributes
        .iter()
        .find(|(own, _)| own == key)
        .map(|(_, value)| value.clone())
}

/// One schema attribute, with the widget its kind takes.
fn typed_row(
    ui: &mut egui::Ui,
    builder: &mut UiBuilder,
    origin: u32,
    spec: &AttributeSpec,
    value: Option<String>,
    catalog: &[AssetCatalogEntry],
) {
    match spec.kind {
        AttributeKind::Bool => {
            label(ui, spec.key, spec.hint, value.is_some());
            let mut on = value.as_deref().unwrap_or(spec.default) != "false";
            if ui.checkbox(&mut on, "").changed() {
                builder.set_attribute(origin, spec.key, if on { "true" } else { "false" });
            }
            unset(ui, builder, origin, spec.key, value.is_some());
        }
        AttributeKind::Number => {
            label(ui, spec.key, spec.hint, value.is_some());
            let mut number: f32 = value
                .as_deref()
                .unwrap_or(spec.default)
                .parse()
                .unwrap_or_default();
            if ui
                .add(egui::DragValue::new(&mut number).speed(0.05))
                .changed()
            {
                builder.set_attribute(origin, spec.key, &number.to_string());
            }
            unset(ui, builder, origin, spec.key, value.is_some());
        }
        AttributeKind::Template => template_row(ui, builder, origin, spec, value, catalog),
        AttributeKind::Action => {
            super::actions::row(ui, builder, origin, spec, value, catalog);
            return;
        }
        AttributeKind::Text | AttributeKind::Binding => {
            let hint = match spec.kind {
                AttributeKind::Binding => "Component.field",
                _ => spec.default,
            };
            text_row(ui, builder, origin, spec.key, value, hint);
            return;
        }
    }
    ui.end_row();
}

fn text_row(
    ui: &mut egui::Ui,
    builder: &mut UiBuilder,
    origin: u32,
    key: &str,
    value: Option<String>,
    hint: &str,
) {
    label(ui, key, "", value.is_some());
    let mut text = value.clone().unwrap_or_default();
    if ui.add(text_edit(&mut text, hint)).changed() {
        builder.set_attribute(origin, key, &text);
    }
    unset(ui, builder, origin, key, value.is_some());
    ui.end_row();
}

/// A `.kxml` of the project through the editor's asset picker, written relative to the file.
fn template_row(
    ui: &mut egui::Ui,
    builder: &mut UiBuilder,
    origin: u32,
    spec: &AttributeSpec,
    value: Option<String>,
    catalog: &[AssetCatalogEntry],
) {
    label(ui, spec.key, spec.hint, value.is_some());
    let folder = builder.path.parent().unwrap_or(Path::new("")).to_path_buf();
    let written = |path: &Path| relative(&folder, path).to_string_lossy().replace('\\', "/");
    let current = value.as_deref().and_then(|value| {
        catalog
            .iter()
            .find(|entry| entry.type_name == SOURCE_TYPE && written(&entry.path) == value)
            .map(|entry| entry.guid)
    });
    let picked = crate::widgets::asset_picker(
        ui,
        ("ui_builder_src", origin),
        current,
        SOURCE_TYPE,
        catalog,
        Some(builder.source),
    );
    match picked {
        Some(Some(guid)) => {
            if let Some(entry) = catalog.iter().find(|entry| entry.guid == guid) {
                builder.set_attribute(origin, spec.key, &written(&entry.path));
            }
        }
        Some(None) => builder.set_attribute(origin, spec.key, ""),
        None => {}
    }
    unset(ui, builder, origin, spec.key, value.is_some());
}

/// Bold when the file sets it, as Unity marks an attribute that overrides its default.
pub(super) fn label(ui: &mut egui::Ui, key: &str, hint: &str, set: bool) {
    let text = egui::RichText::new(key).monospace();
    let response = ui.label(if set { text.strong() } else { text.weak() });
    if !hint.is_empty() {
        response.on_hover_text(hint);
    }
}

pub(super) fn unset(ui: &mut egui::Ui, builder: &mut UiBuilder, origin: u32, key: &str, set: bool) {
    if !set {
        ui.label("");
        return;
    }
    if ui
        .small_button("↺")
        .on_hover_text("Remove it from the element: it reads its default")
        .clicked()
    {
        builder.set_attribute(origin, key, "");
    }
}

fn text_edit<'a>(text: &'a mut String, hint: &'a str) -> egui::TextEdit<'a> {
    egui::TextEdit::singleline(text)
        .hint_text(hint)
        .desired_width(f32::INFINITY)
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
            .add_enabled(
                !key.trim().is_empty() && !value.is_empty(),
                egui::Button::new("Add"),
            )
            .on_hover_text("Any attribute, as the file writes it")
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

/// `to` from inside `folder`, with `..` where they part.
fn relative(folder: &Path, to: &Path) -> PathBuf {
    let from: Vec<Component> = folder.components().collect();
    let target: Vec<Component> = to.components().collect();
    let shared = from.iter().zip(&target).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in shared..from.len() {
        out.push("..");
    }
    for part in &target[shared..] {
        out.push(part);
    }
    out
}
