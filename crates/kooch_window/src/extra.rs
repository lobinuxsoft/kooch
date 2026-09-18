//! Windows beyond the main one (#1196).
//!
//! 🔴 Only the event loop can create a window, and systems never see it: a system asks here and the
//! loop answers on the next frame. Whoever holds the `Arc` owns the window — dropping it closes it.

use std::sync::{Arc, Weak};

use winit::window::{Window, WindowAttributes, WindowId};

/// Requests for extra windows, and the windows made from them.
#[derive(Default)]
pub struct ExtraWindows {
    requests: Vec<(u64, WindowAttributes)>,
    created: Vec<(u64, Arc<Window>)>,
    /// Weak: the loop routes their events, but must not keep a closed one alive.
    live: Vec<(WindowId, Weak<Window>)>,
}

impl ExtraWindows {
    /// Asks the loop for a window. It arrives in [`Self::take_created`] under the same `key`.
    pub fn request(&mut self, key: u64, attrs: WindowAttributes) {
        self.requests.push((key, attrs));
    }

    /// The windows created since the last call, with the key each was requested under.
    pub fn take_created(&mut self) -> Vec<(u64, Arc<Window>)> {
        std::mem::take(&mut self.created)
    }

    /// Whether a window for `key` was asked for and has not been handed out yet.
    pub fn is_pending(&self, key: u64) -> bool {
        self.requests.iter().any(|(k, _)| *k == key) || self.created.iter().any(|(k, _)| *k == key)
    }

    pub(crate) fn take_requests(&mut self) -> Vec<(u64, WindowAttributes)> {
        std::mem::take(&mut self.requests)
    }

    pub(crate) fn push_created(&mut self, key: u64, window: Arc<Window>) {
        self.live.push((window.id(), Arc::downgrade(&window)));
        self.created.push((key, window));
    }

    /// The live window behind `id`, forgetting any that were dropped.
    pub(crate) fn find(&mut self, id: WindowId) -> Option<Arc<Window>> {
        self.live.retain(|(_, weak)| weak.strong_count() > 0);
        self.live
            .iter()
            .find(|(live, _)| *live == id)
            .and_then(|(_, weak)| weak.upgrade())
    }
}

#[cfg(test)]
mod tests;
