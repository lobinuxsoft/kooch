//! Keyboard navigation over the Inspector's component sections.

use kooch_ecs::component::ComponentId;

/// The Inspector's keyboard state.
#[derive(Default)]
pub(crate) struct InspectorNav {
    /// Which component section the cursor is on.
    pub(crate) cursor: Option<ComponentId>,
    /// The sections drawn last frame, in order.
    pub(crate) rows: Vec<ComponentId>,
    /// A section the keyboard asked to open or close.
    pub(crate) toggle: Option<(ComponentId, bool)>,
    /// Set when the cursor moves, so the view scrolls to follow.
    pub(crate) scroll_to_cursor: bool,
}

impl InspectorNav {
    fn index(&self) -> Option<usize> {
        let cursor = self.cursor?;
        self.rows.iter().position(|row| *row == cursor)
    }

    /// Moves the cursor by `delta` sections, clamped.
    pub(crate) fn step(&mut self, delta: isize) {
        if self.rows.is_empty() {
            self.cursor = None;
            return;
        }
        let last = self.rows.len() as isize - 1;
        let next = match self.index() {
            Some(i) => (i as isize + delta).clamp(0, last),
            None => 0,
        };
        self.cursor = Some(self.rows[next as usize]);
        self.scroll_to_cursor = true;
    }

    /// Asks for the section under the cursor to open or close.
    pub(crate) fn set_open(&mut self, open: bool) {
        match self.cursor {
            Some(cursor) if self.index().is_some() => {
                self.toggle = Some((cursor, open));
                self.scroll_to_cursor = true;
            }
            // No cursor yet: the first arrow puts one somewhere instead of
            // being swallowed.
            _ => self.step(0),
        }
    }

    /// Whether `component` is where the cursor is.
    pub(crate) fn is_cursor(&self, component: ComponentId) -> bool {
        self.cursor == Some(component)
    }

    /// Takes the pending toggle, if it names this component.
    pub(crate) fn take_toggle_for(&mut self, component: ComponentId) -> Option<bool> {
        match self.toggle {
            Some((c, open)) if c == component => {
                self.toggle = None;
                Some(open)
            }
            _ => None,
        }
    }

    /// Reads the arrows, if this panel owns them this frame.
    pub(crate) fn handle_keyboard(&mut self, ui: &egui::Ui) {
        self.scroll_to_cursor = false;
        if ui.memory(|m| m.focused().is_some()) {
            return;
        }

        let (up, down, left, right) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
            )
        });

        if up {
            self.step(-1);
        }
        if down {
            self.step(1);
        }
        if right {
            self.set_open(true);
        }
        if left {
            self.set_open(false);
        }
    }
}

#[cfg(test)]
mod tests;
