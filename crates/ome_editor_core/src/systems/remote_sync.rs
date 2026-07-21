//! Drives the remote session each frame.
//!
//! This is what makes remote mode *visible*: it advances the connect
//! handshake, re-pulls the project's entity snapshot on a cadence, and
//! feeds it to [`RemoteMirror`](crate::remote_mirror::RemoteMirror),
//! which rebuilds it in the editor's own ECS. Every downstream
//! consumer — World panel, Inspector, viewport render — then works
//! against that ECS with the ordinary local machinery, unaware the data
//! came off a socket.
//!
//! Edits travel the other way, through
//! [`remote_edit`](crate::actions) — never by mutating the mirror.

use ome_core::resource::Resources;

use crate::remote_session::{ConnectionState, RemoteState};

/// Frames between snapshot pulls while connected.
///
/// The project owns the world and may mutate it every frame (gameplay
/// systems, physics), so the mirror is a poll, not a subscription. Twice
/// a second at 60 fps keeps the editor responsive to external change
/// without spending a synchronous HTTP round-trip per frame.
const REFRESH_INTERVAL_FRAMES: u32 = 30;

/// Per-frame cadence bookkeeping for [`remote_sync_system`].
#[derive(Default)]
pub(crate) struct RemoteSyncState {
    /// Frames since the last snapshot pull.
    frames: u32,
    /// Whether the failure of the current session was already reported,
    /// so a dead project logs once instead of every frame.
    failure_reported: bool,
}

/// Advances the handshake, refreshes the snapshot, and re-applies the
/// mirror. No-op in local mode (no session).
pub(crate) fn remote_sync_system(resources: &mut Resources) {
    // Taken out of Resources: the mirror mutates the ECS through the
    // same `Resources` it lives in.
    let Some(mut state) = resources.remove::<RemoteState>() else {
        return;
    };
    let mut sync = resources.remove::<RemoteSyncState>().unwrap_or_default();

    sync_state(&mut state, &mut sync, resources);

    resources.insert(sync);
    resources.insert(state);
}

/// The body of [`remote_sync_system`], with both resources in hand.
fn sync_state(state: &mut RemoteState, sync: &mut RemoteSyncState, resources: &mut Resources) {
    let RemoteState { session, mirror } = state;
    let Some(session) = session.as_mut() else {
        return;
    };

    let before = session.state();
    let after = session.poll_ready();

    // The handshake pulls the first snapshot itself, so a transition to
    // Connected is immediately mirrorable.
    let just_connected = after == ConnectionState::Connected && before != after;

    match after {
        ConnectionState::Connecting => return,
        ConnectionState::Failed => {
            if !sync.failure_reported {
                sync.failure_reported = true;
                tracing::error!("remote project exited before connecting");
            }
            return;
        }
        ConnectionState::Connected => {}
    }

    if !just_connected {
        sync.frames += 1;
        if sync.frames < REFRESH_INTERVAL_FRAMES {
            return;
        }
        sync.frames = 0;
        session.refresh();
    }

    mirror.apply(session.snapshot(), resources);
}
