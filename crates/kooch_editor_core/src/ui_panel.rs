//! A `kooch_ui` document as an editor panel (#1776): laid out in points, drawn by the game's UI pass
//! into a texture egui shows, and driven by egui's pointer. Menus, drag payloads and text fields
//! stay egui, placed over the node a gesture reports.

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::asset_loader::AssetServer;
use kooch_core::dynamic::{DataAssetTypes, EditorHandlers};
use kooch_core::gpu::GpuContext;
use kooch_core::resource::Resources;
use kooch_ecs::UiDrawList;
use kooch_render::ui::UiPass;
use kooch_ui::style::{STATE_ACTIVE, STATE_HOVER};
use kooch_ui::taffy::{AvailableSpace, Size};
use kooch_ui::{GlyphAtlas, UiFonts, UiNode, UiTree};

use crate::state::{EditorOverlay, EditorTab};
use crate::viewport::ViewportTarget;

/// Every editor panel drawn by `kooch_ui`. Taken out of `Resources` for the frame, like the views.
#[derive(Default)]
pub(crate) struct UiPanels {
    /// The project's data asset types by short name, for the Asset Browser's menu.
    pub data_types: Vec<String>,
    /// Out of `Resources` for the frame, like the panels: what a click runs.
    pub handlers: Option<EditorHandlers>,
    pub asset_browser: UiPanel,
    /// The project's panels open in the dock or a window, by their `.kxml` (#1791).
    pub project: Vec<(Guid, UiPanel)>,
    /// Every panel the project offers, for the Window menu: each `.kxml` under `assets/editor/`.
    pub offered: Vec<(Guid, String)>,
    /// The editor's icon font is in the shared `UiFonts`.
    icons: bool,
}

static PHOSPHOR: &[u8] = include_bytes!("../assets/fonts/Phosphor.ttf");

/// The family name inside `Phosphor.ttf`: what a panel's sheet names to draw `crate::icons`.
pub(crate) const ICON_FAMILY: &str = "Phosphor-Bold";

impl UiPanels {
    fn each(&mut self) -> impl Iterator<Item = &mut UiPanel> {
        std::iter::once(&mut self.asset_browser)
            .chain(self.project.iter_mut().map(|(_, panel)| panel))
    }

    pub(crate) fn project_mut(&mut self, guid: Guid) -> Option<&mut UiPanel> {
        self.project
            .iter_mut()
            .find(|(own, _)| *own == guid)
            .map(|(_, panel)| panel)
    }

    /// The open panel of `guid`, and what its `on:click`s can run.
    pub(crate) fn project_shown(
        &mut self,
        guid: Guid,
    ) -> (Option<&mut UiPanel>, Option<&EditorHandlers>) {
        let panel = self.project.iter_mut().find(|(own, _)| *own == guid);
        (panel.map(|(_, panel)| panel), self.handlers.as_ref())
    }

    pub(crate) fn name_of(&self, guid: Guid) -> Option<&str> {
        self.offered
            .iter()
            .find(|(own, _)| *own == guid)
            .map(|(_, name)| name.as_str())
    }

    /// Keeps `data_types` in step with what the project declared; allocates only when it moved.
    pub(crate) fn sync_data_types(&mut self, types: Option<&DataAssetTypes>) {
        let declared = types.map(DataAssetTypes::all).unwrap_or_default();
        let names = declared
            .iter()
            .map(|schema| schema.type_name.rsplit("::").next().unwrap_or_default());
        if !self.data_types.iter().map(String::as_str).eq(names.clone()) {
            self.data_types = names.map(str::to_owned).collect();
        }
    }

    pub(crate) fn prepare(&mut self, gpu: &GpuContext, renderer: &mut egui_wgpu::Renderer) {
        for panel in self.each() {
            panel.prepare(gpu, renderer);
        }
    }

    pub(crate) fn render(&mut self, gpu: &GpuContext) {
        for panel in self.each() {
            panel.render(gpu);
        }
    }
}

/// Before the render: each shown panel laid out at the size it was shown at last frame.
pub(crate) fn ui_panels_system(resources: &mut Resources) {
    let Some(mut panels) = resources.remove::<UiPanels>() else {
        return;
    };
    project_panels(resources, &mut panels);
    if let Some(fonts) = resources.get_mut::<UiFonts>() {
        if !std::mem::replace(&mut panels.icons, true) {
            fonts.register(PHOSPHOR);
        }
        for panel in panels.each() {
            panel.update(fonts);
        }
    }
    resources.insert(panels);
}

/// Builds the open project panels from their files, and keeps them in step when saved. A tab whose
/// file is gone from the open project is closed.
fn project_panels(resources: &mut Resources, panels: &mut UiPanels) {
    let Some(root) = resources
        .get::<crate::project_state::ProjectState>()
        .and_then(|state| state.active_project.as_ref())
        .map(|project| project.root_path.clone())
    else {
        return;
    };
    let open: Vec<Guid> = resources
        .get::<EditorOverlay>()
        .map(|overlay| {
            let docked = overlay.dock_state.iter_all_tabs().map(|(_, tab)| *tab);
            let windows = overlay.windows.detached.iter().map(|window| window.tab);
            docked
                .chain(windows)
                .filter_map(|tab| match tab {
                    EditorTab::Project(guid) => Some(guid),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let mut server = resources.remove::<AssetServer>();
    let mut gone = Vec::new();
    if let Some(database) = resources.get::<AssetDatabase>() {
        panels.offered = offered(database, &root);
        panels.project.retain(|(guid, _)| open.contains(guid));
        for guid in open {
            let Some(path) = database.entry(guid).map(|entry| entry.path.clone()) else {
                gone.push(guid);
                continue;
            };
            if panels.project_mut(guid).is_none() {
                panels.project.push((guid, UiPanel::default()));
            }
            let Some(panel) = panels.project_mut(guid) else {
                continue;
            };
            let style = database.guid_for(&path.with_extension("kcss"));
            let refreshed =
                kooch_ui::refresh_tree(database, server.as_mut(), &mut panel.tree, guid, style);
            if refreshed == kooch_ui::Refreshed::Rebuilt {
                panel.forget_pointer();
            }
        }
    }
    if let Some(server) = server {
        resources.insert(server);
    }
    if !gone.is_empty()
        && let Some(overlay) = resources.get_mut::<EditorOverlay>()
    {
        let closed =
            |tab: &EditorTab| matches!(tab, EditorTab::Project(guid) if gone.contains(guid));
        overlay.dock_state.retain_tabs(|tab| !closed(tab));
        overlay
            .windows
            .detached
            .retain(|window| !closed(&window.tab));
    }
}

/// Each `.kxml` under the project's `assets/editor/`, by name: Unity's `Editor` folder.
fn offered(database: &AssetDatabase, root: &std::path::Path) -> Vec<(Guid, String)> {
    let absolute = root.join("assets").join("editor");
    let relative = std::path::Path::new("assets").join("editor");
    let mut offered: Vec<(Guid, String)> = database
        .path_iter()
        .filter(|(path, _)| path.starts_with(&absolute) || path.starts_with(&relative))
        .filter(|(path, _)| path.extension().is_some_and(|ext| ext == "kxml"))
        .map(|(path, guid)| {
            let name = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned());
            (guid, name.unwrap_or_default())
        })
        .collect();
    offered.sort_by(|a, b| a.1.cmp(&b.1));
    offered
}

/// What the pointer did to a panel's document this frame, as the nodes it did it on: the nearest
/// one with an `on:click` or a control, or the node itself.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct UiGestures {
    pub hovered: Option<UiNode>,
    pub clicked: Option<UiNode>,
    pub double_clicked: Option<UiNode>,
    pub secondary: Option<UiNode>,
    pub drag_started: Option<UiNode>,
}

#[derive(Default)]
pub(crate) struct UiPanel {
    /// What the panel shows; its owner builds it and keeps it until it changes.
    pub tree: Option<UiTree>,
    atlas: GlyphAtlas,
    list: UiDrawList,
    target: Option<ViewportTarget>,
    pass: Option<UiPass>,
    /// Physical pixels and pixels per point it was shown at; `None` while not drawn.
    request: Option<((u32, u32), f32)>,
    /// The size `list` was built for: the target matches it only once resized.
    drawn: Option<(u32, u32)>,
    /// `list` changed since the target last drew it.
    fresh: bool,
    texture: Option<egui::TextureId>,
    hovered: Option<UiNode>,
    pressed: Option<UiNode>,
    /// Where the panel's top-left was on screen last shown: what turns rects into egui's.
    origin: egui::Pos2,
}

impl UiPanel {
    /// The node a gesture at screen `pos` would land on, for what egui knows and the panel does
    /// not see: a drag from another panel ending over it.
    pub(crate) fn node_at(&self, pos: egui::Pos2) -> Option<UiNode> {
        let point = glam::Vec2::new(pos.x - self.origin.x, pos.y - self.origin.y);
        target(self.tree.as_ref()?, point)
    }

    /// `node`'s rect on screen as last laid out, cut to what its ancestors show.
    pub(crate) fn screen_rect(&self, node: UiNode) -> Option<egui::Rect> {
        let tree = self.tree.as_ref()?;
        let rect = tree.rect(node)?;
        let clip = tree.clip(node)?;
        let min = rect.min.max(glam::Vec2::new(clip[0], clip[1]));
        let max = (rect.min + rect.size).min(glam::Vec2::new(clip[2], clip[3]));
        let offset = self.origin.to_vec2();
        Some(egui::Rect::from_min_max(
            egui::pos2(min.x, min.y) + offset,
            egui::pos2(max.x, max.y) + offset,
        ))
    }

    /// A new document: what the pointer held was the old one's.
    pub(crate) fn set_tree(&mut self, tree: UiTree) {
        self.tree = Some(tree);
        self.forget_pointer();
    }

    fn forget_pointer(&mut self) {
        self.hovered = None;
        self.pressed = None;
    }

    /// The node held under the pointer: a binding leaves it to the hand while it drags.
    pub(crate) fn pressed(&self) -> Option<UiNode> {
        self.pressed
    }

    fn update(&mut self, fonts: &mut UiFonts) {
        let (Some(((width, height), ppp)), Some(tree)) = (self.request, self.tree.as_mut()) else {
            return;
        };
        tree.restyle();
        let laid = tree.layout(
            Size {
                width: AvailableSpace::Definite(width as f32 / ppp),
                height: AvailableSpace::Definite(height as f32 / ppp),
            },
            fonts,
        );
        let rastered = tree.rasterize(ppp, fonts, &mut self.atlas);
        if !laid && !rastered && self.drawn == Some((width, height)) {
            return;
        }
        let physical = glam::UVec2::new(width, height);
        // No shaders: a panel redraws only when it changes, so `time` would not run.
        let shaders = kooch_ui::UiShaders::default();
        kooch_ui::draw_tree(
            tree,
            &mut self.atlas,
            &shaders,
            ppp,
            physical,
            &mut self.list,
        );
        self.drawn = Some((width, height));
        self.fresh = true;
    }

    fn prepare(&mut self, gpu: &GpuContext, renderer: &mut egui_wgpu::Renderer) {
        if let Some((size, _)) = self.request.take() {
            match self.target.as_mut() {
                Some(target) => {
                    target.request_size(size);
                    target.resize_if_needed(gpu.device(), renderer);
                }
                None => {
                    self.target = Some(ViewportTarget::new(
                        gpu.device(),
                        renderer,
                        gpu.format(),
                        size,
                    ))
                }
            }
        }
        self.texture = self.target.as_ref().map(ViewportTarget::texture_id);
    }

    /// Only when the list changed: the texture keeps what it last drew.
    fn render(&mut self, gpu: &GpuContext) {
        let (Some(target), Some(drawn)) = (self.target.as_ref(), self.drawn) else {
            return;
        };
        // A resized target is blank until drawn again.
        if target.size() != drawn || !std::mem::take(&mut self.fresh) {
            return;
        }
        let pass = self
            .pass
            .get_or_insert_with(|| UiPass::new(gpu.device(), gpu.format()));
        let mut encoder = gpu
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ui_panel"),
            });
        pass.draw(
            gpu.device(),
            gpu.queue(),
            &mut encoder,
            target.view(),
            drawn,
            &mut self.list,
            true,
        );
        gpu.queue().submit(Some(encoder.finish()));
    }

    /// Fills what is left of `ui`. Rects are in points from the panel's top-left: `rect_of` turns
    /// them into screen space for what egui draws over a node.
    pub(crate) fn show(&mut self, ui: &mut egui::Ui) -> (egui::Response, UiGestures) {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let ppp = ui.ctx().pixels_per_point();
        let size = (
            (rect.width() * ppp).round().max(1.0) as u32,
            (rect.height() * ppp).round().max(1.0) as u32,
        );
        if self.drawn != Some(size) {
            // Resizing takes a frame to land: ask for it.
            ui.ctx().request_repaint();
        }
        self.request = Some((size, ppp));
        self.origin = rect.min;
        if let (Some(texture), Some(drawn)) = (self.texture, self.drawn) {
            // At the size it was drawn, a frame behind a resize: cut, never stretched.
            let shown = egui::Rect::from_min_size(
                rect.min,
                egui::vec2(drawn.0 as f32, drawn.1 as f32) / ppp,
            );
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter()
                .with_clip_rect(rect)
                .image(texture, shown, uv, egui::Color32::WHITE);
        }
        let mut gestures = UiGestures::default();
        let Some(tree) = self.tree.as_mut() else {
            return (response, gestures);
        };
        let local = |pos: egui::Pos2| glam::Vec2::new(pos.x - rect.min.x, pos.y - rect.min.y);
        let over = response
            .hover_pos()
            .and_then(|pos| target(tree, local(pos)));
        if let Some(point) = response.hover_pos().map(local) {
            let wheel = ui.input(|input| input.smooth_scroll_delta);
            // egui's delta moves the content; the offset moves the view, the other way.
            if wheel != egui::Vec2::ZERO
                && tree.scroll_at(point, glam::Vec2::new(-wheel.x, -wheel.y))
            {
                ui.ctx().request_repaint();
            }
        }
        let pressed = match response.is_pointer_button_down_on() {
            true => self.pressed.or(over),
            false => None,
        };
        let changes = [
            (self.hovered, over, STATE_HOVER),
            (self.pressed, pressed, STATE_ACTIVE),
        ];
        for (before, after, bit) in changes {
            if before == after {
                continue;
            }
            if let Some(node) = before {
                tree.set_state_up(node, bit, false);
            }
            if let Some(node) = after {
                tree.set_state_up(node, bit, true);
            }
            ui.ctx().request_repaint();
        }
        self.hovered = over;
        self.pressed = pressed;
        gestures.hovered = over;
        // A held slider follows the pointer; a toggle flips on its click.
        if let (Some(node), Some(pos)) = (pressed, response.interact_pointer_pos())
            && tree.slide_to(node, local(pos).x)
        {
            ui.ctx().request_repaint();
        }
        gestures.clicked = over.filter(|_| response.clicked());
        if let Some(node) = gestures.clicked
            && tree.flip(node)
        {
            ui.ctx().request_repaint();
        }
        gestures.double_clicked = over.filter(|_| response.double_clicked());
        gestures.secondary = over.filter(|_| response.secondary_clicked());
        if response.drag_started() {
            let origin = ui.input(|input| input.pointer.press_origin());
            gestures.drag_started = origin.and_then(|pos| target(tree, local(pos)));
        }
        (response, gestures)
    }
}

/// The node a gesture at `point` lands on: the nearest with an `on:click` or a control, else the
/// one hit.
fn target(tree: &UiTree, point: glam::Vec2) -> Option<UiNode> {
    let hit = tree.pick(point)?;
    let mut current = Some(hit);
    while let Some(node) = current {
        if tree.interactive(node) {
            return Some(node);
        }
        current = tree.parent(node);
    }
    Some(hit)
}
