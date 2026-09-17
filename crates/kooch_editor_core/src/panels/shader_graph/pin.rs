//! A pin and its name as one element (#1170): the pin draws its own name beside the rect egui-snarl
//! placed it at, so the two cannot come apart. It is coloured by how much of the wire it reads, and
//! explains itself on hover (#1159).
//!
//! 🔴 Measured headless before this: a name laid out in its row landed wherever egui-snarl's sizing of
//! that row put it — "sine" at x 711 against a node ending at 707, each row 15 points further. Drawn by
//! the pin, the name is positioned from the pin's own rect and nothing else.

use egui::{Color32, Painter, PopupAnchor, Rect, Style, Tooltip, pos2};
use egui_snarl::ui::{PinInfo, PinWireInfo, SnarlPin, SnarlStyle};

use crate::shader_graph::Width;

/// Between a pin and its name.
const GAP: f32 = 5.0;

#[derive(Clone, Copy, Hash)]
pub(super) enum Side {
    /// On the left of a node: the name reads to the pin's right.
    Input,
    /// On the right of a node: the name reads to the pin's left.
    Output,
}

pub(super) struct NamedPin {
    /// The name with its width, `speed (2)`.
    pub label: String,
    pub side: Side,
    pub width: Width,
    /// What it takes or gives, on hover.
    pub about: &'static str,
    /// Unique to the pin, for its tooltip.
    pub id: egui::Id,
}

/// Unity Shader Graph's port colours, so a wire's width reads the same to anyone coming from it.
pub(super) fn colour(width: Width) -> Color32 {
    match width {
        Width::One => Color32::from_rgb(0x84, 0xE4, 0xE7),
        Width::Two => Color32::from_rgb(0x9A, 0xEF, 0x92),
        Width::Three => Color32::from_rgb(0xF6, 0xFF, 0x9A),
        Width::Four => Color32::from_rgb(0xFB, 0xCB, 0xF4),
        Width::Any => Color32::from_gray(0xB0),
    }
}

/// `speed (2)`: the name and how many components it reads.
pub(super) fn label(name: &str, width: Width) -> String {
    format!("{name}{}", width.suffix())
}

impl SnarlPin for NamedPin {
    fn draw(
        self,
        snarl_style: &SnarlStyle,
        style: &Style,
        rect: Rect,
        painter: &Painter,
    ) -> PinWireInfo {
        let wire =
            PinInfo::circle()
                .with_fill(colour(self.width))
                .draw(snarl_style, style, rect, painter);
        let text = style.visuals.text_color();
        let galley = painter.layout_no_wrap(
            self.label.clone(),
            egui::TextStyle::Body.resolve(style),
            text,
        );
        let size = galley.size();
        let x = match self.side {
            Side::Output => rect.left() - GAP - size.x,
            Side::Input => rect.right() + GAP,
        };
        let name = Rect::from_min_size(pos2(x, rect.center().y - size.y / 2.0), size);
        painter.galley(name.min, galley, text);
        if hovered(painter, rect.union(name), style) {
            Tooltip::always_open(
                painter.ctx().clone(),
                painter.layer_id(),
                self.id,
                PopupAnchor::Pointer,
            )
            .show(|ui| {
                ui.strong(&self.label);
                ui.weak(self.width.describe());
                ui.label(self.about);
            });
        }
        wire
    }
}

/// The pointer resting over `area`, in the painter's zoomed layer, as long as a tooltip waits.
fn hovered(painter: &Painter, area: Rect, style: &Style) -> bool {
    let ctx = painter.ctx();
    let Some(pointer) = ctx.pointer_hover_pos() else {
        return false;
    };
    let local = ctx
        .layer_transform_from_global(painter.layer_id())
        .map_or(pointer, |to_local| to_local * pointer);
    area.contains(local)
        && ctx.input(|i| {
            !i.pointer.any_down()
                && i.pointer.time_since_last_movement() >= style.interaction.tooltip_delay
        })
}

/// Room for a pin's name in its row, so the node grows to hold it. The name itself is drawn by the pin.
pub(super) fn reserve(ui: &mut egui::Ui, name: &str) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let size = ui
        .painter()
        .layout_no_wrap(name.to_owned(), font, egui::Color32::WHITE)
        .size();
    ui.allocate_space(egui::vec2(size.x + GAP, size.y));
}
