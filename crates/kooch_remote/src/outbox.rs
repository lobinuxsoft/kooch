//! What the host has to tell the editor without being asked (#57): the protocol only answers, so
//! signals ride the `Moved` reply the editor already pulls every Play frame.

use kooch_core::resource::Resources;

use crate::protocol::HostSignal;

/// Far above a frame's worth: the editor drains every pull, so this only binds when nobody does.
const CAPACITY: usize = 256;

/// Signals waiting for the next `ListMoved`.
#[derive(Default)]
pub struct HostOutbox(Vec<HostSignal>);

impl HostOutbox {
    /// Queues a signal, dropping the oldest once full — with no editor pulling, it would only grow.
    pub fn push(&mut self, name: impl Into<String>, payload: serde_json::Value) {
        if self.0.len() >= CAPACITY {
            self.0.remove(0);
        }
        self.0.push(HostSignal {
            name: name.into(),
            payload,
        });
    }
}

/// Everything queued since the last reply.
pub(crate) fn drain(resources: &mut Resources) -> Vec<HostSignal> {
    resources
        .get_mut::<HostOutbox>()
        .map(|outbox| std::mem::take(&mut outbox.0))
        .unwrap_or_default()
}
