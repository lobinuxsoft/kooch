//! The element's classes as chips, and the sheet's rules that reach it, as Unity's Builder lists
//! an element's style classes and matching selectors.

use super::inspector::{label, unset};
use crate::icons;
use crate::ui_builder::UiBuilder;

pub(super) fn row(ui: &mut egui::Ui, builder: &mut UiBuilder, origin: u32, written: &str) {
    label(
        ui,
        "class",
        "Each one is matched by the sheet's .name selectors",
        !written.is_empty(),
    );
    let sheet = builder.known.sheet.as_ref();
    let defined: Vec<String> = sheet
        .map(|sheet| sheet.parsed.classes().into_iter().map(Into::into).collect())
        .unwrap_or_default();
    let sheet_name = sheet.map(|sheet| sheet.name.clone());
    let classes: Vec<&str> = written.split_whitespace().collect();
    let mut next: Option<Vec<String>> = None;
    let owned = || {
        classes
            .iter()
            .map(|class| (*class).to_owned())
            .collect::<Vec<_>>()
    };
    ui.horizontal_wrapped(|ui| {
        for (at, class) in classes.iter().enumerate() {
            let text = egui::RichText::new(format!(".{class} {}", icons::X)).monospace();
            let found = defined.iter().any(|own| own == class);
            let (text, hint) = match (&sheet_name, found) {
                (Some(name), false) => (
                    text.color(ui.visuals().warn_fg_color),
                    format!("No rule of {name} names it. Click to remove"),
                ),
                _ => (text, "Click to remove".to_owned()),
            };
            if ui.small_button(text).on_hover_text(hint).clicked() {
                let mut kept = owned();
                kept.remove(at);
                next = Some(kept);
            }
        }
        let id = ui.make_persistent_id(("ui_builder_new_class", origin));
        let mut typed: String = ui.data(|data| data.get_temp(id)).unwrap_or_default();
        let field = ui.add(
            egui::TextEdit::singleline(&mut typed)
                .hint_text("+ class")
                .desired_width(72.0),
        );
        if field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            let class = typed.trim().trim_start_matches('.');
            if !class.is_empty() && !class.contains(char::is_whitespace) {
                let mut added = owned();
                added.push(class.to_owned());
                next = Some(added);
            }
            typed.clear();
        }
        ui.data_mut(|data| data.insert_temp(id, typed));
        crate::widgets::search_menu(
            ui,
            ("ui_builder_classes", origin),
            Some(14.0),
            String::new(),
            |ui, needle| {
                let mut shown = 0;
                for class in &defined {
                    if classes.contains(&class.as_str())
                        || (!needle.is_empty() && !class.to_lowercase().contains(needle))
                    {
                        continue;
                    }
                    if ui.selectable_label(false, format!(".{class}")).clicked() {
                        let mut added = owned();
                        added.push(class.clone());
                        next = Some(added);
                        ui.close();
                    }
                    shown += 1;
                }
                if shown == 0 {
                    ui.weak("(no other class in the sheet)");
                }
            },
        );
    });
    unset(ui, builder, origin, "class", !written.is_empty());
    ui.end_row();
    if let Some(classes) = next {
        builder.set_attribute(origin, "class", &classes.join(" "));
    }
}

/// The rules of the canvas's sheet that reach the element, strongest first, each with its
/// declarations as written.
pub(super) fn matching(ui: &mut egui::Ui, builder: &UiBuilder, origin: u32) {
    let (Some(sheet), Some(file)) = (&builder.known.sheet, &builder.known.file) else {
        ui.weak("The canvas wears the default sheet.");
        return;
    };
    if origin as usize >= file.elements.len() {
        return;
    }
    let rules = kooch_ui::matching_rules(file, &sheet.parsed, origin as usize);
    if rules.is_empty() {
        ui.weak(format!("No rule of {} reaches it.", sheet.name));
        return;
    }
    for &index in rules.iter().rev() {
        let rule = &sheet.parsed.rules[index];
        let selectors: Vec<String> = rule.selectors.iter().map(ToString::to_string).collect();
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(selectors.join(", "))
                    .monospace()
                    .strong(),
            );
            ui.weak(format!("{}:{}", sheet.name, rule.line));
        });
        let body = block(&sheet.text, rule.line);
        if !body.is_empty() {
            ui.label(egui::RichText::new(body).monospace().weak());
        }
        ui.add_space(4.0);
    }
}

/// The declarations between the braces of the rule that starts on `line`, one per line.
fn block(text: &str, line: u32) -> String {
    let start: usize = text
        .split_inclusive('\n')
        .take(line.saturating_sub(1) as usize)
        .map(str::len)
        .sum();
    let rest = &text[start.min(text.len())..];
    let Some(open) = rest.find('{') else {
        return String::new();
    };
    let inner = &rest[open + 1..];
    let inner = &inner[..inner.find('}').unwrap_or(inner.len())];
    inner
        .split(';')
        .map(str::trim)
        .filter(|declaration| !declaration.is_empty())
        .collect::<Vec<_>>()
        .join(";\n")
}
