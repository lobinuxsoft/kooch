//! The selected node's pins and fields, beside the canvas, where they have room (#1944).

use egui_snarl::InPinId;

use super::editors::{curve_editor, pin_field};
use super::fields::{node_fields, trim_pins};
use super::viewer::has_fields;
use crate::panels::inspector::AssetCatalogEntry;
use crate::shader_graph::{Graph, Node};

/// One node selected: its title, what it does, each pin with its value and doc, and its fields.
/// Edited in place, like the node itself, so the graph's frame diff makes it one undo step.
pub(super) fn draw_node_inspector(
    ui: &mut egui::Ui,
    graph: &mut Graph,
    snarl_id: egui::Id,
    catalog: &[AssetCatalogEntry],
) {
    let selected = egui_snarl::ui::get_selected_nodes(snarl_id, ui.ctx());
    let [id] = selected[..] else {
        ui.weak(match selected.len() {
            0 => "Select a node to see its pins and fields here.",
            _ => "Several nodes selected: select one to see it here.",
        });
        return;
    };
    let wired: Vec<usize> = graph
        .wires()
        .filter(|(_, to)| to.node == id)
        .map(|(_, to)| to.input)
        .collect();
    let Some(node) = graph.get_node_mut(id) else {
        return;
    };
    ui.strong(node.title());
    ui.label(node.about());

    let names: Vec<String> = node.inputs().iter().map(|n| (*n).to_owned()).collect();
    if !names.is_empty() {
        ui.separator();
        ui.strong("Pins");
    }
    for (input, name) in names.iter().enumerate() {
        let (width, about) = node
            .input_docs()
            .get(input)
            .copied()
            .unwrap_or((node.pin_width(input), ""));
        ui.horizontal(|ui| {
            ui.label(name);
            if wired.contains(&input) {
                ui.weak("wired");
            } else {
                pin_field(ui, node, InPinId { node: id, input }, width);
            }
        });
        if !about.is_empty() {
            ui.label(egui::RichText::new(about).small().weak());
        }
    }

    if let Node::Curve(curve) = &mut node.node {
        ui.separator();
        ui.strong("Curve");
        curve_editor(ui, id, curve, ui.available_width());
    } else if has_fields(&node.node) {
        ui.separator();
        ui.strong("Fields");
        node_fields(ui, &mut node.node, catalog);
    }
    trim_pins(graph, id);
}
