//! The inline style as Unity's Builder shows it: a foldout per category, a typed field per
//! property. A set property is bold and ↺ unsets it; a value its field cannot show stays text.

use kooch_render::material::{ParamKind, ShaderParam};
use kooch_ui::style::{
    CATEGORIES, InlineStyle, PARAM_PREFIX, PropertyKind, PropertySpec, parse_color,
};

use std::path::Path;

use kooch_core::Guid;

use crate::panels::inspector::{AssetCatalogEntry, AssetSource};
use crate::ui_builder::UiBuilder;

/// What the asset fields pick from, and the folder their `url()`s are relative to.
struct Assets<'a> {
    fonts: &'a [(Guid, String)],
    catalog: &'a [AssetCatalogEntry],
    folder: &'a Path,
    /// The project's `assets/`, where a build merges the engine's: an engine image is written there.
    project: Option<&'a Path>,
}

pub(super) fn draw(
    ui: &mut egui::Ui,
    builder: &mut UiBuilder,
    origin: u32,
    written: &str,
    catalog: &[AssetCatalogEntry],
) {
    let mut style = InlineStyle::parse(written);
    let mut changed = false;
    let folder = builder.path.parent().unwrap_or(Path::new("")).to_path_buf();
    let project = catalog
        .iter()
        .find(|entry| entry.guid == builder.source)
        .and_then(|entry| entry.path.ancestors().nth(entry.name.components().count()));
    let assets = Assets {
        fonts: &builder.fonts,
        catalog,
        folder: &folder,
        project,
    };
    for category in &CATEGORIES {
        let set = category
            .properties
            .iter()
            .any(|spec| style.get(spec.name).is_some());
        let title = match set {
            true => egui::RichText::new(format!("{}*", category.title)).strong(),
            false => egui::RichText::new(category.title),
        };
        egui::CollapsingHeader::new(title)
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
                super::inspector::fields_grid(
                    ui,
                    ("ui_builder_style_grid", category.title),
                    STYLE_LABELS,
                )
                .striped(true)
                .show(ui, |ui| {
                    for spec in category.properties {
                        changed |= row(ui, &mut style, origin, spec, &assets);
                    }
                });
            });
    }
    let params = builder.params_of(origin);
    if !params.is_empty() {
        egui::CollapsingHeader::new("Material")
            .id_salt("ui_builder_material")
            .default_open(true)
            .show(ui, |ui| {
                super::inspector::fields_grid(ui, "ui_builder_material_grid", STYLE_LABELS)
                    .striped(true)
                    .show(ui, |ui| {
                        for param in params {
                            changed |= param_row(ui, &mut style, param);
                        }
                    });
            });
    }
    if changed {
        builder.set_attribute(origin, "style", &style.write());
    }
}

/// A row per declared parameter, written as `-kooch-param-<name>`; unset, the shader's default.
fn param_row(ui: &mut egui::Ui, style: &mut InlineStyle, param: &ShaderParam) -> bool {
    let property = format!("{PARAM_PREFIX}{}", param.name.to_ascii_lowercase());
    let value = style.get(&property);
    super::inspector::label(ui, &param.name, "", value.is_some());
    let mut edited = param_field(ui, param, value.as_deref()).map(Some);
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
    style.set(&property, value.as_deref());
    true
}

fn param_field(ui: &mut egui::Ui, param: &ShaderParam, value: Option<&str>) -> Option<String> {
    match param.kind {
        ParamKind::Float | ParamKind::Int => {
            let mut number = value
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(param.default[0]);
            let response = match param.range {
                Some([min, max]) => ui.add(egui::Slider::new(&mut number, min..=max)),
                None => ui.add(egui::DragValue::new(&mut number).speed(0.01)),
            };
            response.changed().then(|| decimal(number))
        }
        ParamKind::Color => {
            let rgba = value.and_then(parse_color).unwrap_or(param.default);
            let mut bytes = rgba.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
            ui.color_edit_button_srgba_unmultiplied(&mut bytes)
                .changed()
                .then(|| hex(bytes))
        }
        // Space-separated, as the `.kcss` writes them.
        _ => text(ui, value),
    }
}

/// Room the property names and ↺ take: `border-bottom-width` is the longest.
const STYLE_LABELS: f32 = 185.0;

fn row(
    ui: &mut egui::Ui,
    style: &mut InlineStyle,
    origin: u32,
    spec: &PropertySpec,
    assets: &Assets,
) -> bool {
    let value = style.get(spec.name);
    super::inspector::label(ui, spec.name, spec.hint, value.is_some());
    let mut edited = field(ui, origin, spec, value.as_deref(), assets).map(Some);
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
    if spec.name.ends_with("gap") && value.as_deref().is_some_and(|value| value.contains('%')) {
        ui.label("");
        ui.colored_label(
            ui.visuals().warn_fg_color,
            "A % gap is a share of this element's own size: without a set height (or width) it is \
             laid out with none, then overflows. Set the size, or use px.",
        );
        ui.label("");
        ui.end_row();
    }
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
    assets: &Assets,
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
        PropertyKind::Font => font(ui, origin, value, assets),
        PropertyKind::Image => url(
            ui,
            origin,
            value,
            assets,
            std::any::type_name::<kooch_render::Image>(),
        ),
        PropertyKind::Shader => quoted_path(ui, origin, value, assets),
    }
}

/// A font file, written as the family inside it.
fn font(ui: &mut egui::Ui, origin: u32, value: Option<&str>, assets: &Assets) -> Option<String> {
    let family = value.map(|value| value.trim().trim_matches(['"', '\'']));
    let current = family.and_then(|family| {
        assets
            .fonts
            .iter()
            .find(|(_, own)| own == family)
            .map(|(guid, _)| *guid)
    });
    let picked = picker(
        ui,
        ("ui_builder_font", origin),
        current,
        kooch_ui::FONT_TYPE,
        assets,
    )?;
    let guid = picked?;
    // A file not registered yet has no family to write.
    let (_, family) = assets.fonts.iter().find(|(own, _)| *own == guid)?;
    Some(format!("\"{family}\""))
}

/// An image asset, written as `url()` relative to the open file.
fn url(
    ui: &mut egui::Ui,
    origin: u32,
    value: Option<&str>,
    assets: &Assets,
    kind: &str,
) -> Option<String> {
    let written = value.and_then(|value| {
        let inner = value.trim().strip_prefix("url(")?.strip_suffix(')')?;
        Some(inner.trim().trim_matches(['"', '\'']).to_owned())
    });
    let current = written.and_then(|written| find(assets, kind, &written));
    let picked = picker(ui, ("ui_builder_url", origin), current, kind, assets)?;
    Some(match picked {
        Some(guid) => format!("url(\"{}\")", relative(assets, guid)?),
        None => "none".to_owned(),
    })
}

/// A `kind: ui` shader, written as its quoted path relative to the open file.
fn quoted_path(
    ui: &mut egui::Ui,
    origin: u32,
    value: Option<&str>,
    assets: &Assets,
) -> Option<String> {
    let kind = kooch_render::material::SHADER_TYPE_NAME;
    let written = value.map(|value| value.trim().trim_matches(['"', '\'']).to_owned());
    let current = written.and_then(|written| find(assets, kind, &written));
    let picked = picker(ui, ("ui_builder_shader", origin), current, kind, assets)?;
    Some(format!("\"{}\"", relative(assets, picked?)?))
}

fn picker(
    ui: &mut egui::Ui,
    id: (&str, u32),
    current: Option<Guid>,
    kind: &str,
    assets: &Assets,
) -> Option<Option<Guid>> {
    crate::widgets::asset_picker(ui, id, current, kind, assets.catalog, None)
}

/// A project file shadows the engine's of the same name, as it does at runtime.
fn find(assets: &Assets, kind: &str, written: &str) -> Option<Guid> {
    assets
        .catalog
        .iter()
        .filter(|entry| {
            entry.type_name == kind && relative(assets, entry.guid).as_deref() == Some(written)
        })
        .min_by_key(|entry| entry.source != AssetSource::Project)
        .map(|entry| entry.guid)
}

fn relative(assets: &Assets, guid: Guid) -> Option<String> {
    let entry = assets.catalog.iter().find(|entry| entry.guid == guid)?;
    let path = match (entry.source, assets.project) {
        (AssetSource::Engine, Some(project)) => project.join(&entry.name),
        _ => entry.path.clone(),
    };
    let path = super::inspector::relative(assets.folder, &path);
    Some(path.to_string_lossy().replace('\\', "/"))
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
