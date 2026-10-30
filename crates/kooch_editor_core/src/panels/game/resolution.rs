//! The Game view's resolution: the panel's own size, or a fixed one scaled to fit it, the way
//! Unity's Game view offers the display's resolutions.

use egui::{Rect, Vec2};

/// What the Game view renders at, and what it can offer.
#[derive(Default)]
pub(crate) struct GameResolution {
    /// `None` renders at the panel's size.
    pub choice: Option<[u32; 2]>,
    /// The display's sizes, one per size, largest first.
    pub sizes: Vec<[u32; 2]>,
    /// Where last frame's image was drawn, so the cursor can be sent in the game's pixels (#96).
    pub image: Option<GameImage>,
}

/// The game image on the editor's window, in physical pixels, and the target it shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GameImage {
    pub min: glam::Vec2,
    pub size: glam::Vec2,
    pub target: glam::Vec2,
}

impl GameImage {
    /// A window position in the target's pixels; `None` off the image.
    pub fn map(&self, window: glam::Vec2) -> Option<glam::Vec2> {
        let local = (window - self.min) / self.size;
        let inside = local.cmpge(glam::Vec2::ZERO).all() && local.cmplt(glam::Vec2::ONE).all();
        inside.then_some(local * self.target)
    }
}

/// One entry per size: a monitor reports each size once per refresh rate.
pub(crate) fn sizes(modes: &kooch_core::window_mode::DisplayModes) -> Vec<[u32; 2]> {
    let mut sizes: Vec<[u32; 2]> = modes.modes.iter().map(|m| [m.width, m.height]).collect();
    sizes.dedup();
    sizes
}

/// The largest rect of `size`'s aspect that fits `panel`, centred in it.
pub(crate) fn fit(panel: Rect, size: [u32; 2]) -> Rect {
    let aspect = size[0] as f32 / size[1].max(1) as f32;
    let width = panel.width().min(panel.height() * aspect);
    Rect::from_center_size(panel.center(), Vec2::new(width, width / aspect))
}

fn label(choice: Option<[u32; 2]>) -> String {
    match choice {
        Some([w, h]) => format!("{w}×{h}"),
        None => "Free".to_owned(),
    }
}

/// The dropdown, beside the View menu.
pub(crate) fn picker(ui: &mut egui::Ui, origin: egui::Pos2, resolution: &mut GameResolution) {
    let rect = Rect::from_min_size(origin + Vec2::new(104.0, 8.0), Vec2::new(150.0, 26.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(20, 20, 24, 200))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(&mut child, |ui| {
            egui::ComboBox::from_id_salt("game_resolution")
                .selected_text(label(resolution.choice))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut resolution.choice, None, "Free")
                        .on_hover_text("Render at the panel's size");
                    for &size in &resolution.sizes {
                        ui.selectable_value(&mut resolution.choice, Some(size), label(Some(size)));
                    }
                })
                .response
                .on_hover_text(
                    "What the game renders at. A fixed size keeps its aspect and is scaled to fit \
                     the panel.",
                );
        });
}

#[cfg(test)]
mod tests;
