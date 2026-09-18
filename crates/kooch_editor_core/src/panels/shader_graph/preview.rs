//! The preview column: the graph's shader on a shape, beside the nodes (#1159).

use super::PreviewView;

/// The preview column: the shader on a shape, and which shape that is.
pub(super) fn draw_preview(ui: &mut egui::Ui, preview: PreviewView<'_>) {
    const SIDE: f32 = 220.0;

    // `Panel::right` rather than `SidePanel`: egui 0.35 folded the four side/top/bottom builders
    // into one `Panel`, as `input_map` already found out.
    let mut request = crate::viewport::PreviewRequest {
        primitive: preview.primitive,
        size: None,
    };
    egui::Panel::right("shader_graph_preview")
        .resizable(true)
        .default_size(SIDE)
        .size_range(140.0..=640.0)
        .show(ui, |ui| {
            ui.add_space(4.0);
            egui::ComboBox::from_id_salt("shader_preview_shape")
                .selected_text(shape_name(preview.primitive))
                .show_ui(ui, |ui| {
                    for (index, (name, _)) in
                        kooch_render::mesh::Primitive::CANONICAL.iter().enumerate()
                    {
                        if ui
                            .selectable_label(index == preview.primitive, display_name(name))
                            .clicked()
                        {
                            request.primitive = index;
                        }
                    }
                });
            ui.add_space(4.0);

            let side = ui.available_width();
            ui.add(egui::Image::new(egui::load::SizedTexture::new(
                preview.texture,
                egui::vec2(side, side),
            )));
            // 🔴 Only once the column has settled. Every frame of a drag would drop and re-create the
            // target's textures; until the button is released the last image is stretched instead.
            if !ui.input(|input| input.pointer.any_down()) {
                request.size = Some((side * ui.ctx().pixels_per_point()).round() as u32);
            }

            // 🔴 A graph is edited node by node, and most of those moments do not compile. The panel
            // says so instead of showing the last shader that did, which would be a lie about what
            // is on the canvas.
            match preview.refusal {
                Some(note) if note == crate::viewport::shader_preview::POST_NOTE => {
                    ui.label(egui::RichText::new(note).small());
                }
                Some(why) => {
                    ui.colored_label(ui.visuals().error_fg_color, "This graph does not compile");
                    ui.label(egui::RichText::new(why).small());
                }
                None => {}
            }
        });

    // Asked for every frame the panel is drawn, so nothing renders behind a closed tab.
    *preview.request = Some(request);
}

fn shape_name(index: usize) -> String {
    kooch_render::mesh::Primitive::CANONICAL
        .get(index)
        .map(|(name, _)| display_name(name))
        .unwrap_or_default()
}

/// `uv_sphere` reads as "Uv Sphere" in a menu, not as a file name.
fn display_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (index, word) in name.split('_').enumerate() {
        if index > 0 {
            out.push(' ');
        }
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out
}
