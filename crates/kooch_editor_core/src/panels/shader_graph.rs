//! Shader Graph panel — the nodes, and the file they write (#1159).

use egui_snarl::ui::{PinInfo, SnarlViewer, SnarlWidget};
use egui_snarl::{InPin, NodeId, OutPin, Snarl};

use crate::shader_graph::{Graph, Node};

/// What the panel needs to draw one frame.
pub(crate) struct ShaderGraphView<'a> {
    /// The open graph, edited in place: `egui-snarl` moves nodes and wires while it draws them.
    pub graph: Option<&'a mut Graph>,
    /// The file it generates, for the header.
    pub path: Option<&'a std::path::Path>,
    /// Whether the graph diverges from that file.
    pub dirty: bool,
}

/// What the panel asks for.
pub(crate) enum ShaderGraphAction {
    /// Generate the shader and write it.
    Save,
}

/// Every node the first slice offers, in the order the menu lists them.
fn palette() -> Vec<Node> {
    vec![
        Node::Uv,
        Node::WorldNormal,
        Node::WorldPosition,
        Node::ViewDirection,
        Node::Constant([1.0; 4]),
        Node::Param {
            name: "value".to_owned(),
            width: 1,
            color: false,
            default: [0.0; 4],
        },
        Node::Texture {
            name: "map".to_owned(),
            fallback: "white".to_owned(),
        },
        Node::Add,
        Node::Multiply,
        Node::Mix,
        Node::Dot,
        Node::Power,
        Node::Saturate,
        Node::Output,
    ]
}

/// Draws the panel. Returns what the user asked for.
pub(crate) fn draw_shader_graph_content(
    ui: &mut egui::Ui,
    view: ShaderGraphView<'_>,
) -> Vec<ShaderGraphAction> {
    let mut actions = Vec::new();
    let Some(graph) = view.graph else {
        ui.weak("No shader graph open.");
        ui.label("Create one in the Asset Browser: New Shader Graph, or open a generated .shader.");
        return actions;
    };

    ui.horizontal(|ui| {
        if let Some(path) = view.path {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            ui.label(format!("{name}{}", if view.dirty { " *" } else { "" }));
        }
        if ui
            .button("Save")
            .on_hover_text("Generate the .shader from this graph and write it")
            .clicked()
        {
            actions.push(ShaderGraphAction::Save);
        }
        ui.weak("Right-click the background to add a node.");
    });
    ui.separator();

    SnarlWidget::new()
        .id(egui::Id::new("shader_graph"))
        .show(graph, &mut Viewer, ui);
    actions
}

/// How the nodes draw and connect.
struct Viewer;

impl SnarlViewer<Node> for Viewer {
    fn title(&mut self, node: &Node) -> String {
        node.title()
    }

    fn inputs(&mut self, node: &Node) -> usize {
        node.inputs().len()
    }

    fn outputs(&mut self, node: &Node) -> usize {
        usize::from(node.has_output())
    }

    fn show_input(
        &mut self,
        pin: &InPin,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) -> impl egui_snarl::ui::SnarlPin + 'static {
        let name = snarl
            .get_node(pin.id.node)
            .and_then(|node| node.inputs().get(pin.id.input).copied())
            .unwrap_or("in");
        ui.label(name);
        PinInfo::circle()
    }

    fn show_output(
        &mut self,
        pin: &OutPin,
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) -> impl egui_snarl::ui::SnarlPin + 'static {
        // A value node carries its own value, so the node itself is where it is edited.
        if let Some(node) = snarl.get_node_mut(pin.id.node) {
            match node {
                Node::Constant(value) => {
                    ui.horizontal(|ui| {
                        for component in value.iter_mut() {
                            ui.add(crate::numeric::drag(component).speed(0.01));
                        }
                    });
                }
                Node::Param {
                    name,
                    width,
                    color,
                    default,
                } => {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(name).desired_width(70.0));
                        ui.add(egui::DragValue::new(width).range(1..=4).prefix("x"));
                        // Only four wide: the hint it writes is a `vec4<f32>` one, and it cannot
                        // outlive a width the user narrowed under it.
                        *color &= *width == 4;
                        if *width == 4 {
                            ui.checkbox(color, "color");
                        }
                    });
                    ui.horizontal(|ui| {
                        for component in default.iter_mut().take(*width as usize) {
                            ui.add(crate::numeric::drag(component).speed(0.01));
                        }
                    });
                }
                Node::Texture { name, fallback } => {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(name).desired_width(70.0));
                        for option in ["white", "black", "normal"] {
                            if ui.selectable_label(fallback == option, option).clicked() {
                                *fallback = option.to_owned();
                            }
                        }
                    });
                }
                _ => {
                    ui.label("out");
                }
            }
        }
        PinInfo::circle()
    }

    /// One wire per input: a second one replaces the first, which is what every graph tool does.
    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<Node>) {
        for &remote in &to.remotes {
            snarl.disconnect(remote, to.id);
        }
        snarl.connect(from.id, to.id);
    }

    fn has_graph_menu(&mut self, _pos: egui::Pos2, _snarl: &mut Snarl<Node>) -> bool {
        true
    }

    fn show_graph_menu(&mut self, pos: egui::Pos2, ui: &mut egui::Ui, snarl: &mut Snarl<Node>) {
        ui.label("Add node");
        for node in palette() {
            if ui.button(node.title()).clicked() {
                snarl.insert_node(pos, node);
                ui.close();
            }
        }
    }

    fn has_node_menu(&mut self, _node: &Node) -> bool {
        true
    }

    fn show_node_menu(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) {
        if ui
            .button(format!("{} Remove", crate::icons::TRASH))
            .clicked()
        {
            snarl.remove_node(node);
            ui.close();
        }
    }
}
