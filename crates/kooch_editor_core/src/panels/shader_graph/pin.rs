//! A pin and its name as one element (#1170): the pin draws its own name beside the rect egui-snarl
//! placed it at, so the two cannot come apart.
//!
//! 🔴 Measured headless before this: a name laid out in its row landed wherever egui-snarl's sizing of
//! that row put it — "sine" at x 711 against a node ending at 707, each row 15 points further. Drawn by
//! the pin, the name is positioned from the pin's own rect and nothing else.

use egui::{Painter, Rect, Style, pos2};
use egui_snarl::ui::{PinInfo, PinWireInfo, SnarlPin, SnarlStyle};

/// Between a pin and its name.
const GAP: f32 = 5.0;

#[derive(Clone, Copy)]
pub(super) enum Side {
    /// On the left of a node: the name reads to the pin's right.
    Input,
    /// On the right of a node: the name reads to the pin's left.
    Output,
}

pub(super) struct NamedPin {
    /// `None` beside a node's fields, which name the pin themselves.
    pub name: Option<&'static str>,
    pub side: Side,
}

impl SnarlPin for NamedPin {
    fn draw(
        self,
        snarl_style: &SnarlStyle,
        style: &Style,
        rect: Rect,
        painter: &Painter,
    ) -> PinWireInfo {
        let wire = PinInfo::circle().draw(snarl_style, style, rect, painter);
        if let Some(name) = self.name {
            let colour = style.visuals.text_color();
            let galley = painter.layout_no_wrap(
                name.to_owned(),
                egui::TextStyle::Body.resolve(style),
                colour,
            );
            let size = galley.size();
            let x = match self.side {
                Side::Output => rect.left() - GAP - size.x,
                Side::Input => rect.right() + GAP,
            };
            painter.galley(pos2(x, rect.center().y - size.y / 2.0), galley, colour);
        }
        wire
    }
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
