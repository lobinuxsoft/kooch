//! Widgets shared by more than one panel.

/// Height of one list row, in points.
pub(crate) fn row_height(ui: &egui::Ui) -> f32 {
    use egui::NumExt as _;
    let line = ui.text_style_height(&egui::TextStyle::Button);
    (line + 2.0 * ui.spacing().button_padding.y).at_least(ui.spacing().interact_size.y)
}

/// A list row that spans the full width of its panel.
pub(crate) struct SelectableRow {
    text: egui::WidgetText,
    selected: bool,
    sense: egui::Sense,
    dimmed: bool,
}

impl SelectableRow {
    pub(crate) fn new(text: impl Into<egui::WidgetText>) -> Self {
        Self {
            text: text.into(),
            selected: false,
            sense: egui::Sense::click(),
            dimmed: false,
        }
    }

    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Widen what the row reacts to. Click and drag on one response
    /// rather than two widgets, so a drag overlay cannot steal the click
    /// that would have selected the row.
    pub(crate) fn sense(mut self, sense: egui::Sense) -> Self {
        self.sense = sense;
        self
    }

    /// Fade the text — for an item currently being dragged somewhere
    /// else, so the row it came from reads as "in transit".
    pub(crate) fn dimmed(mut self, dimmed: bool) -> Self {
        self.dimmed = dimmed;
        self
    }

    pub(crate) fn show(self, ui: &mut egui::Ui) -> egui::Response {
        let button_padding = ui.spacing().button_padding;
        let wrap_width = ui.available_width() - 2.0 * button_padding.x;
        let galley = self.text.into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            wrap_width,
            egui::TextStyle::Button,
        );

        let desired_size = egui::vec2(ui.available_width(), row_height(ui));
        let (rect, resp) = ui.allocate_at_least(desired_size, self.sense);

        if ui.is_rect_visible(rect) {
            // Left-aligned and vertically centred, stated rather than inherited from the layout:
            // now that the row is as wide as the panel, asking the layout where to put the text
            // would centre a short name in the middle of a wide row.
            let inner = rect.shrink2(button_padding);
            let text_pos = egui::pos2(inner.left(), inner.center().y - galley.size().y * 0.5);
            let visuals = ui.style().interact_selectable(&resp, self.selected);
            if self.selected || resp.hovered() || resp.highlighted() || resp.has_focus() {
                ui.painter().rect(
                    rect.expand(visuals.expansion),
                    visuals.corner_radius,
                    visuals.bg_fill,
                    visuals.bg_stroke,
                    egui::StrokeKind::Inside,
                );
            }
            let mut text_color = visuals.text_color();
            if self.dimmed {
                text_color = text_color.gamma_multiply(0.3);
            }
            ui.painter().galley(text_pos, galley, text_color);
        }
        resp
    }
}

#[cfg(test)]
mod tests;
