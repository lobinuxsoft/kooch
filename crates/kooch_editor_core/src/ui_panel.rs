//! A `kooch_ui` document as an editor panel (#1776): laid out in points, drawn by the game's UI pass
//! into a texture egui shows, and driven by egui's pointer. Menus, drag payloads and text fields
//! stay egui, placed over the node a gesture reports.

use kooch_core::gpu::GpuContext;
use kooch_core::resource::Resources;
use kooch_ecs::UiDrawList;
use kooch_render::ui::UiPass;
use kooch_ui::style::{STATE_ACTIVE, STATE_HOVER};
use kooch_ui::taffy::{AvailableSpace, Size};
use kooch_ui::{GlyphAtlas, UiFonts, UiNode, UiTree};

use crate::viewport::ViewportTarget;

/// Every editor panel drawn by `kooch_ui`. Taken out of `Resources` for the frame, like the views.
#[derive(Default)]
pub(crate) struct UiPanels {
    pub asset_browser: UiPanel,
}

impl UiPanels {
    fn each(&mut self) -> [&mut UiPanel; 1] {
        [&mut self.asset_browser]
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
    if let Some(fonts) = resources.get_mut::<UiFonts>() {
        for panel in panels.each() {
            panel.update(fonts);
        }
    }
    resources.insert(panels);
}

/// What the pointer did to a panel's document this frame, as the nodes it did it on: the nearest
/// one with an `on:click`, or the node itself.
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
}

impl UiPanel {
    /// A new document: what the pointer held was the old one's.
    pub(crate) fn set_tree(&mut self, tree: UiTree) {
        self.tree = Some(tree);
        self.hovered = None;
        self.pressed = None;
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
        kooch_ui::draw_tree(tree, &mut self.atlas, ppp, physical, &mut self.list);
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
        gestures.clicked = over.filter(|_| response.clicked());
        gestures.double_clicked = over.filter(|_| response.double_clicked());
        gestures.secondary = over.filter(|_| response.secondary_clicked());
        if response.drag_started() {
            let origin = ui.input(|input| input.pointer.press_origin());
            gestures.drag_started = origin.and_then(|pos| target(tree, local(pos)));
        }
        (response, gestures)
    }
}

/// The node a gesture at `point` lands on: the nearest with an `on:click`, else the one hit.
fn target(tree: &UiTree, point: glam::Vec2) -> Option<UiNode> {
    let hit = tree.pick(point)?;
    let mut current = Some(hit);
    while let Some(node) = current {
        if tree.click(node).is_some() {
            return Some(node);
        }
        current = tree.parent(node);
    }
    Some(hit)
}
