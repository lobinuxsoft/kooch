//! Shader Graph panel — the nodes, and the file they write (#1159).

use egui::emath::TSTransform;
use egui_snarl::ui::{PinInfo, SnarlViewer, SnarlWidget};
use egui_snarl::{InPin, NodeId, OutPin, Snarl};

use crate::panels::inspector::AssetCatalogEntry;
use crate::shader_graph::{
    BLEND_MODES, Category, Graph, Node, TEXTURE_FALLBACKS, arrange, palette,
};

/// What the panel needs to draw one frame.
pub(crate) struct ShaderGraphView<'a> {
    /// The open graph, edited in place: `egui-snarl` moves nodes and wires while it draws them.
    pub graph: Option<&'a mut Graph>,
    /// The file it generates, for the header.
    pub path: Option<&'a std::path::Path>,
    /// Whether the graph diverges from that file.
    pub dirty: bool,
    /// The shader on a shape, beside the nodes that wrote it.
    pub preview: PreviewView<'a>,
    /// Every asset, for a texture node to pick the image its preview samples.
    pub catalog: &'a [AssetCatalogEntry],
}

/// The preview column: what to draw, on what, and how to ask for something else.
pub(crate) struct PreviewView<'a> {
    pub texture: egui::TextureId,
    /// Which of `Primitive::CANONICAL` is showing.
    pub primitive: usize,
    /// Why the shader did not build, if it did not.
    pub refusal: Option<&'a str>,
    /// The shape the panel wants next frame. `Some` also means the panel was drawn at all, which
    /// is what keeps the preview from rendering behind a tab nobody opened.
    pub request: &'a mut Option<crate::viewport::PreviewRequest>,
}

/// What the panel asks for.
pub(crate) enum ShaderGraphAction {
    /// Generate the shader and write it.
    Save,
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
    draw_preview(ui, view.preview);

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
        if ui
            .button("Arrange")
            .on_hover_text("Lay the nodes out left to right, in layers")
            .clicked()
        {
            arrange(graph);
        }
        let mut showing = minimap_shown(ui);
        if ui.checkbox(&mut showing, "Minimap").changed() {
            show_minimap(ui, showing);
        }
        ui.weak("Right-click the background to add a node.");
    });
    ui.separator();

    // 🔴 The graph follows its panel. `egui-snarl` keeps the view in GLOBAL screen coordinates and
    // re-anchors it never, so a panel that moved left its graph behind — off screen, with nothing
    // to do but pan blindly looking for it (#1167).
    let panel = ui.max_rect();
    let mut viewer = Viewer {
        catalog: view.catalog,
        drift: drift(ui, panel),
        look_at: taken_look(ui),
        panel,
        transform: TSTransform::IDENTITY,
    };

    SnarlWidget::new()
        .id(egui::Id::new("shader_graph"))
        .show(graph, &mut viewer, ui);

    if minimap_shown(ui)
        && let Some(at) = crate::panels::graph_minimap::draw(ui, panel, graph, viewer.transform)
    {
        look_at(ui, at);
    }
    actions
}

/// How wide a node's fields are. Narrow on purpose: a node reads as a column, not a long row.
const FIELD: f32 = 110.0;

/// A parameter's name, on a line of its own.
fn name_field(ui: &mut egui::Ui, name: &mut String) {
    ui.add(egui::TextEdit::singleline(name).desired_width(FIELD));
}

/// One number per line, labelled by the component it is.
fn components(ui: &mut egui::Ui, values: &mut [f32]) {
    for (value, axis) in values.iter_mut().zip(["x", "y", "z", "w"]) {
        ui.add(
            crate::numeric::drag(value)
                .speed(0.01)
                .prefix(format!("{axis} ")),
        );
    }
}

/// One choice from a short list, as a dropdown: a row of buttons made every such node wide.
fn choice(ui: &mut egui::Ui, id: &str, current: &mut String, options: &[&str]) {
    egui::ComboBox::from_id_salt(id)
        .width(FIELD)
        .selected_text(current.as_str())
        .show_ui(ui, |ui| {
            for option in options {
                if ui.selectable_label(current == option, *option).clicked() {
                    *current = (*option).to_owned();
                }
            }
        });
}

/// A float or a whole number, top to bottom: its name, whether it has a range, the range, and its
/// starting value — a slider inside the range, as the material's Inspector will draw it.
fn number_editor(
    ui: &mut egui::Ui,
    name: &mut String,
    default: &mut f32,
    range: &mut Option<[f32; 2]>,
    whole: bool,
) {
    let step = if whole { 1.0 } else { 0.01 };
    let decimals = if whole { 0 } else { 2 };
    name_field(ui, name);
    let mut ranged = range.is_some();
    if ui.checkbox(&mut ranged, "range").changed() {
        *range = ranged.then_some([0.0, if whole { 10.0 } else { 1.0 }]);
    }
    if let Some([lo, hi]) = range.as_mut() {
        for (bound, label) in [(&mut *lo, "min "), (&mut *hi, "max ")] {
            ui.add(
                egui::DragValue::new(bound)
                    .speed(step)
                    .fixed_decimals(decimals)
                    .prefix(label),
            );
        }
        // 🔴 A range the wrong way round is a slider egui cannot draw; keep it ordered as it is typed.
        if *hi < *lo {
            *hi = *lo;
        }
    }
    match *range {
        Some([lo, hi]) => {
            ui.spacing_mut().slider_width = FIELD - 50.0;
            ui.add(
                egui::Slider::new(default, lo..=hi)
                    .step_by(if whole { 1.0 } else { 0.0 })
                    .fixed_decimals(decimals),
            );
        }
        None => {
            ui.add(
                egui::DragValue::new(default)
                    .speed(step)
                    .fixed_decimals(decimals),
            );
        }
    }
    if whole {
        *default = default.round();
    }
}

/// The preview column: the shader on a shape, and which shape that is.
fn draw_preview(ui: &mut egui::Ui, preview: PreviewView<'_>) {
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
            if let Some(why) = preview.refusal {
                ui.colored_label(ui.visuals().error_fg_color, "This graph does not compile");
                ui.label(egui::RichText::new(why).small());
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

/// How far the panel moved since the last frame — what the view has to travel to stay with it.
fn drift(ui: &egui::Ui, panel: egui::Rect) -> egui::Vec2 {
    let id = egui::Id::new("shader_graph_panel_rect");
    let before = ui.ctx().data(|d| d.get_temp::<egui::Rect>(id));
    ui.ctx().data_mut(|d| d.insert_temp(id, panel));
    match before {
        Some(before) if before.is_finite() && panel.is_finite() => panel.min - before.min,
        _ => egui::Vec2::ZERO,
    }
}

/// Whether the minimap is showing, and where the user asked to look. Both live in egui's own store:
/// how a graph is being *viewed* is not part of the graph, and must never reach the file.
fn minimap_shown(ui: &egui::Ui) -> bool {
    ui.ctx()
        .data(|d| d.get_temp::<bool>(egui::Id::new("shader_graph_minimap")))
        .unwrap_or(true)
}

fn show_minimap(ui: &egui::Ui, showing: bool) {
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::new("shader_graph_minimap"), showing));
}

fn look_at(ui: &egui::Ui, at: egui::Pos2) {
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::new("shader_graph_look_at"), at));
}

/// Takes the pending jump, so it is applied once and not every frame after.
fn taken_look(ui: &egui::Ui) -> Option<egui::Pos2> {
    let id = egui::Id::new("shader_graph_look_at");
    ui.ctx().data_mut(|d| {
        let at = d.get_temp::<egui::Pos2>(id);
        d.remove_temp::<egui::Pos2>(id);
        at
    })
}

/// How the nodes draw and connect.
struct Viewer<'a> {
    /// Every asset, for a texture node's preview image.
    catalog: &'a [AssetCatalogEntry],
    /// How far the panel moved since the last frame.
    drift: egui::Vec2,
    /// Where the minimap asked to look, in graph space.
    look_at: Option<egui::Pos2>,
    /// The panel, for centring a jump.
    panel: egui::Rect,
    /// This frame's view transform, taken back out for the minimap to draw what is on screen.
    transform: TSTransform,
}

impl SnarlViewer<Node> for Viewer<'_> {
    /// The one hook that runs between the pan/zoom handling and the transform being stored, which
    /// is the only place a view can be corrected from outside the widget.
    fn current_transform(&mut self, to_global: &mut TSTransform, _snarl: &mut Snarl<Node>) {
        to_global.translation += self.drift;
        if let Some(at) = self.look_at {
            // That point of the graph, under the middle of the panel.
            to_global.translation =
                self.panel.center().to_vec2() - to_global.scaling * at.to_vec2();
        }
        self.transform = *to_global;
    }

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
        let catalog = self.catalog;
        if let Some(node) = snarl.get_node_mut(pin.id.node) {
            // 🔴 An output pin's row is laid out RIGHT TO LEFT by egui-snarl, so fields added one after
            // another landed in a row, and in reverse. A column of its own is what stacks them.
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| match node {
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
                } => {
                    name_field(ui, name);
                    components(ui, &mut default[..(*width).clamp(2, 4) as usize]);
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
                _ => {
                    ui.label("out");
                }
            });
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

    /// Grouped, because a flat list of forty nodes is a list nobody reads.
    fn show_graph_menu(&mut self, pos: egui::Pos2, ui: &mut egui::Ui, snarl: &mut Snarl<Node>) {
        ui.label("Add node");
        for category in Category::ALL {
            ui.menu_button(category.label(), |ui| {
                for node in palette()
                    .into_iter()
                    .filter(|node| node.category() == category)
                {
                    if ui.button(node.title()).clicked() {
                        snarl.insert_node(pos, node);
                        ui.close();
                    }
                }
            });
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
