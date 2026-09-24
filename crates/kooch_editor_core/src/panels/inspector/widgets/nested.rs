//! A reflected struct inside a field — a list's row (#1209): one labelled widget per member.

use kooch_ecs::reflect::{FieldMeta, ReflectValue};

use super::value_widget::{FieldContext, draw_value_widget};

/// Draws `members` as a small grid. Returns the whole struct when a member changed, so the edit
/// travels up as one value.
pub(super) fn draw_struct(
    ui: &mut egui::Ui,
    members: &[(String, ReflectValue)],
    field: &FieldContext<'_>,
) -> Option<ReflectValue> {
    let mut edited = None;
    egui::Grid::new(ui.id().with("struct"))
        .num_columns(2)
        .spacing([6.0, 2.0])
        .show(ui, |ui| {
            for (index, (name, value)) in members.iter().enumerate() {
                let meta = field.fields.iter().find(|meta| meta.name == name);
                let label = ui.label(name.as_str());
                if let Some(meta) = meta.filter(|meta| !meta.doc.is_empty()) {
                    label.on_hover_text(meta.doc);
                }
                let member = member_context(field, name, meta);
                ui.push_id(name, |ui| {
                    if let Some(new) = draw_value_widget(ui, value, &member) {
                        edited = Some((index, new));
                    }
                });
                ui.end_row();
            }
        });
    let (index, new) = edited?;
    let mut members = members.to_vec();
    members[index].1 = new;
    Some(ReflectValue::Struct(members))
}

/// The member's own presentation from its metadata; the catalogues come from the field around it.
fn member_context<'a>(
    field: &FieldContext<'a>,
    name: &'a str,
    meta: Option<&'static FieldMeta>,
) -> FieldContext<'a> {
    FieldContext {
        name,
        choices: meta.map_or(&[], |meta| meta.choices),
        bits: meta.map_or(&[], |meta| meta.bits),
        layers: meta.is_some_and(|meta| meta.layers),
        layer: meta.is_some_and(|meta| meta.layer),
        layer_labels: field.layer_labels,
        assets: field.assets,
        entities: field.entities,
        requires: meta.map_or("", |meta| meta.requires),
        range: meta.and_then(|meta| meta.range),
        fields: meta.map_or(&[], |meta| meta.fields),
    }
}
