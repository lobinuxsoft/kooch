//! A node's own fields, drawn the same on the canvas and in the inspector (#1944).

use egui_snarl::{InPinId, NodeId, Snarl};

use super::editors::{FIELD, choice, components, name_field, number_editor};
use crate::panels::inspector::AssetCatalogEntry;
use crate::shader_graph::{
    BLEND_MODES, GraphNode, NOISE_BASES, NOISE_FRACTALS, Node, OUTPUT_KINDS, TEXTURE_FALLBACKS,
    VORONOI_METRICS,
};

/// The fields a node carries besides its pins. A value node carries its own value, so this is
/// where it is edited. A curve is not drawn here: it needs a width of its own.
pub(super) fn node_fields(ui: &mut egui::Ui, node: &mut Node, catalog: &[AssetCatalogEntry]) {
    match node {
        Node::Argument { name } | Node::Result { name } => name_field(ui, name),
        Node::Float {
            name,
            default,
            range,
        } => number_editor(ui, name, default, range, false),
        Node::Int {
            name,
            default,
            range,
        } => number_editor(ui, name, default, range, true),
        Node::Vector {
            name,
            width,
            default,
            point,
        } => {
            name_field(ui, name);
            components(ui, &mut default[..(*width).clamp(2, 4) as usize]);
            if *width == 2 {
                ui.checkbox(point, "Point")
                    .on_hover_text("A position across the node, dragged in the UI Builder");
            }
        }
        Node::Color { name, default } => {
            name_field(ui, name);
            ui.color_edit_button_rgba_unmultiplied(default);
        }
        Node::Texture {
            name,
            fallback,
            preview,
        } => {
            name_field(ui, name);
            choice(ui, "fallback", fallback, &TEXTURE_FALLBACKS);
            // Seen in the preview only; a material still starts from `fallback`.
            ui.weak("preview");
            let picked = ui
                .push_id("preview", |ui| {
                    crate::panels::inspector::draw_asset_picker(
                        ui,
                        *preview,
                        crate::panels::inspector::IMAGE_TYPE,
                        catalog,
                    )
                })
                .inner;
            if let Some(kooch_ecs::reflect::ReflectValue::AssetRef { guid, .. }) = picked {
                *preview = guid;
            }
        }
        Node::ConstFloat(value) => {
            ui.add(crate::numeric::drag(value).speed(0.01));
        }
        Node::ConstInt(value) => {
            ui.add(egui::DragValue::new(value).speed(1.0).fixed_decimals(0));
            *value = value.round();
        }
        Node::ConstVector { width, value } => {
            components(ui, &mut value[..(*width).clamp(2, 4) as usize]);
        }
        Node::Constant(value) => components(ui, value),
        Node::ConstColor(value) => {
            ui.color_edit_button_rgba_unmultiplied(value);
        }
        Node::Swizzle { pattern } => {
            ui.add(
                egui::TextEdit::singleline(pattern)
                    .desired_width(FIELD)
                    .hint_text("xyzw"),
            );
        }
        Node::Blend { mode } => choice(ui, "blend", mode, &BLEND_MODES),
        Node::FractalNoise { basis, fractal } => {
            choice(ui, "basis", basis, &NOISE_BASES);
            choice(ui, "fractal", fractal, &NOISE_FRACTALS);
        }
        Node::VoronoiNoise { metric } => choice(ui, "metric", metric, &VORONOI_METRICS),
        Node::ShaderOutput { kind } => choice(ui, "kind", kind, &OUTPUT_KINDS),
        _ => {}
    }
}

/// A kind with fewer pins drops the wires and values of the ones it no longer has.
pub(super) fn trim_pins(snarl: &mut Snarl<GraphNode>, id: NodeId) {
    let count = snarl.get_node(id).map_or(0, |n| n.inputs().len());
    if let Some(node) = snarl.get_node_mut(id) {
        node.pins.retain(|&input, _| input < count);
    }
    let stale: Vec<_> = snarl
        .wires()
        .filter(|(_, to): &(_, InPinId)| to.node == id && to.input >= count)
        .collect();
    for (from, to) in stale {
        snarl.disconnect(from, to);
    }
}
