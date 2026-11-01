//! The inline style as Unity's Builder shows it: a foldout per category, a typed field per
//! property. A set property is bold and ↺ unsets it; a value its field cannot show stays text.

use kooch_ui::style::{CATEGORIES, InlineStyle, PropertyKind, PropertySpec, parse_color};

use crate::ui_builder::UiBuilder;

pub(super) fn draw(ui: &mut egui::Ui, builder: &mut UiBuilder, origin: u32, written: &str) {
    let mut style = InlineStyle::parse(written);
    let mut changed = false;
    for category in &CATEGORIES {
        let set = category
            .properties
            .iter()
            .any(|spec| style.get(spec.name).is_some());
        let title = egui::RichText::new(category.title);
        egui::CollapsingHeader::new(if set { title.strong() } else { title })
            .id_salt(("ui_builder_style", category.title))
            .show(ui, |ui| {
                if set
                    && ui
                        .small_button("↺ Unset all")
                        .on_hover_text("Remove every property of this group from the element")
                        .clicked()
                {
                    for spec in category.properties {
                        style.set(spec.name, None);
                    }
                    changed = true;
                }
                egui::Grid::new(("ui_builder_style_grid", category.title))
                    .num_columns(3)
                    .striped(true)
                    .show(ui, |ui| {
                        for spec in category.properties {
                            changed |= row(ui, &mut style, origin, spec);
                        }
                    });
            });
    }
    if changed {
        builder.set_attribute(origin, "style", &style.write());
    }
}

fn row(ui: &mut egui::Ui, style: &mut InlineStyle, origin: u32, spec: &PropertySpec) -> bool {
    let value = style.get(spec.name);
    let text = egui::RichText::new(spec.name).monospace();
    let label = ui.label(if value.is_some() {
        text.strong()
    } else {
        text.weak()
    });
    if !spec.hint.is_empty() {
        label.on_hover_text(spec.hint);
    }
    let mut edited = field(ui, origin, spec, value.as_deref()).map(Some);
    if value.is_none() {
        ui.label("");
    } else if ui
        .small_button("↺")
        .on_hover_text("Remove it: the sheet decides")
        .clicked()
    {
        edited = Some(None);
    }
    ui.end_row();
    let Some(value) = edited else {
        return false;
    };
    style.set(spec.name, value.as_deref());
    true
}

/// The value written when the field changed.
fn field(
    ui: &mut egui::Ui,
    origin: u32,
    spec: &PropertySpec,
    value: Option<&str>,
) -> Option<String> {
    match spec.kind {
        PropertyKind::Keyword(words) => {
            if value.is_some_and(|value| !words.contains(&value)) {
                return text(ui, value);
            }
            let mut picked = None;
            egui::ComboBox::from_id_salt(("ui_builder_keyword", origin, spec.name))
                .selected_text(value.unwrap_or("—"))
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for word in words {
                        if ui.selectable_label(value == Some(word), *word).clicked() {
                            picked = Some((*word).to_owned());
                        }
                    }
                });
            picked
        }
        PropertyKind::Length { auto } => length(ui, origin, spec.name, value, auto),
        PropertyKind::Number => {
            let Some(mut number) = value.map_or(Some(0.0), |value| value.trim().parse().ok())
            else {
                return text(ui, value);
            };
            let drag = egui::DragValue::new(&mut number).speed(0.05);
            ui.add(drag).changed().then(|| decimal(number))
        }
        PropertyKind::Px => {
            let Some(mut number) = value.map_or(Some(0.0), |value| {
                split_unit(value)
                    .filter(|(_, unit)| *unit == "px")
                    .map(|(n, _)| n)
            }) else {
                return text(ui, value);
            };
            let drag = egui::DragValue::new(&mut number)
                .range(0.0..=f32::MAX)
                .suffix(" px");
            ui.add(drag)
                .changed()
                .then(|| format!("{}px", decimal(number)))
        }
        PropertyKind::Color => {
            let Some(rgba) = value.map_or(Some([1.0; 4]), parse_color) else {
                return text(ui, value);
            };
            let mut bytes = rgba.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
            ui.color_edit_button_srgba_unmultiplied(&mut bytes)
                .changed()
                .then(|| hex(bytes))
        }
        PropertyKind::Text => text(ui, value),
    }
}

/// A number and its unit; `auto` and a bare `0` too.
fn length(
    ui: &mut egui::Ui,
    origin: u32,
    name: &str,
    value: Option<&str>,
    auto: bool,
) -> Option<String> {
    let (mut number, unit) = match value.map(str::trim) {
        None | Some("0") => (0.0, "px"),
        Some("auto") if auto => (0.0, "auto"),
        Some(written) => match split_unit(written) {
            Some(split) => split,
            None => return text(ui, value),
        },
    };
    let units: &[&str] = if auto {
        &["px", "%", "auto"]
    } else {
        &["px", "%"]
    };
    let mut picked = unit;
    let mut changed = false;
    ui.horizontal(|ui| {
        if unit != "auto" {
            let drag = egui::DragValue::new(&mut number).speed(if unit == "%" { 0.5 } else { 1.0 });
            changed |= ui.add(drag).changed();
        }
        egui::ComboBox::from_id_salt(("ui_builder_unit", origin, name))
            .selected_text(unit)
            .width(52.0)
            .show_ui(ui, |ui| {
                for unit in units {
                    ui.selectable_value(&mut picked, *unit, *unit);
                }
            });
    });
    if picked != unit {
        changed = true;
        number = 0.0;
    }
    changed.then(|| match picked {
        "auto" => "auto".to_owned(),
        unit => format!("{}{unit}", decimal(number)),
    })
}

fn split_unit(value: &str) -> Option<(f32, &'static str)> {
    let value = value.trim();
    let (number, unit) = if let Some(number) = value.strip_suffix("px") {
        (number, "px")
    } else {
        (value.strip_suffix('%')?, "%")
    };
    Some((number.trim().parse().ok()?, unit))
}

fn text(ui: &mut egui::Ui, value: Option<&str>) -> Option<String> {
    let mut text = value.unwrap_or_default().to_owned();
    let edit = egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY);
    ui.add(edit).changed().then_some(text)
}

/// Three decimals at most, without trailing zeros.
fn decimal(number: f32) -> String {
    let text = format!("{number:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn hex([r, g, b, a]: [u8; 4]) -> String {
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}
