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

use std::time::{Duration, Instant};

use ome_core::resource::Resources;
use ome_gizmos_handles::HandleSet;

use crate::perf::{EditorPerfStats, RemoteSyncStats};
use crate::remote_session::{ConnectionState, RemoteSession, RemoteState};

/// Time between snapshot pulls while the project sits paused.
///
/// The project owns the world and may mutate it behind the editor's
/// back, so the mirror is a poll, not a subscription. Twice a second
/// keeps the editor responsive to outside change without spending a
/// synchronous round-trip per frame.
///
/// This used to be "every thirtieth frame", which was the same thing
/// only for as long as frames arrived at a fixed rate. Since #656 an
/// idle editor draws roughly four frames a second, and thirty of those
/// is seven and a half seconds — the mirror would have looked frozen.
/// A cadence is a duration; counting frames was always a stand-in for
/// one.
const REFRESH_INTERVAL_IDLE: Duration = Duration::from_millis(500);

/// Cadence bookkeeping for [`remote_sync_system`].
pub(crate) struct RemoteSyncState {
    /// When the last snapshot pull finished, or `None` before the first.
    last_pull: Option<Instant>,
    /// How long to wait between pulls while paused. A field rather than
    /// the constant so a test can ask for every frame without sleeping
    /// through a real half-second.
    idle_interval: Duration,
    /// Whether the failure of the current session was already reported,
    /// so a dead project logs once instead of every frame.
    failure_reported: bool,
}

impl Default for RemoteSyncState {
    fn default() -> Self {
        Self {
            last_pull: None,
            idle_interval: REFRESH_INTERVAL_IDLE,
            failure_reported: false,
        }
    }
}

impl RemoteSyncState {
    /// The same state on a different paused-mode cadence.
    #[cfg(test)]
    pub(crate) fn every_frame() -> Self {
        Self {
            idle_interval: Duration::ZERO,
            ..Self::default()
        }
    }
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
    let RemoteState {
        session,
        mirror,
        playing,
    } = state;
    let Some(session) = session.as_mut() else {
        // Local mode costs nothing remote, and saying so is not the
        // same as saying zero — the HUD hides the section entirely.
        if let Some(stats) = resources.get_mut::<EditorPerfStats>() {
            stats.remote = None;
        }
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
                tracing::error!("remote project exited — use Rebuild to relaunch it");
                // Tear the mirror down. Left standing it would look
                // editable while every edit went nowhere, and a Save
                // would write an empty scene: mirrored entities are
                // ephemeral, so they are excluded from the document.
                *playing = false;
                mirror.clear(resources);
            }
            return;
        }
        ConnectionState::Connected => {}
    }

    // A gizmo drag is an edit the project has not been told about yet —
    // it only goes over the wire when the user lets go. Applying the
    // poll on top of it overwrites the in-progress pose with the
    // project's older one, so the handle snaps back mid-drag; worse, the
    // drag-end no-op guard then reads before == after and drops the edit
    // entirely. The drag owns the local Transform until it ends.
    // `just_connected` still gets through: that is the mirror's very
    // first apply, and there is nothing to select — let alone drag —
    // before it has run.
    if drag_in_flight(resources) && !just_connected {
        // Restart the cadence so the frame after release does not
        // immediately apply the snapshot we just skipped.
        sync.last_pull = Some(Instant::now());
        return;
    }

    // `None` on the handshake frame: the snapshot came from
    // `poll_ready`, so there is no refresh of ours to time — and that
    // snapshot starts the clock, so the first pull of our own is a full
    // interval away rather than one frame later.
    let mut refresh = None;
    if just_connected {
        sync.last_pull = Some(Instant::now());
    } else {
        // Gameplay moves things every tick, so a paused-mode cadence
        // would render as a slideshow. The cost is one round-trip per
        // frame over a local socket; the mirror diffs in place, so a
        // pull that changes nothing structural is just field writes.
        let due = *playing
            || sync
                .last_pull
                .is_none_or(|last| last.elapsed() >= sync.idle_interval);
        if !due {
            return;
        }
        let started = Instant::now();
        session.refresh();
        refresh = Some(started.elapsed());
        sync.last_pull = Some(Instant::now());
    }

    let applying = Instant::now();
    mirror.apply(session.snapshot(), resources);
    record_stats(resources, session, refresh, applying.elapsed());
}

/// Folds this pull's cost into [`EditorPerfStats`] (#645).
///
/// The refresh timing is the editor's own wall clock; the transport /
/// decode split comes off the client, which recorded it during that
/// same call. On the handshake frame there is no refresh of ours to
/// report, so the previous sample's numbers are carried rather than
/// overwritten with a zero that would read as "free".
fn record_stats(
    resources: &mut Resources,
    session: &RemoteSession,
    refresh: Option<Duration>,
    mirror: Duration,
) {
    let ms = |d: Duration| d.as_secs_f32() * 1000.0;
    let call = session.client().last_call_stats();

    let Some(stats) = resources.get_mut::<EditorPerfStats>() else {
        return;
    };
    let previous = stats.remote.unwrap_or_default();
    stats.remote = Some(RemoteSyncStats {
        refresh_ms: refresh.map_or(previous.refresh_ms, ms),
        transport_ms: refresh.map_or(previous.transport_ms, |_| call.transport_us as f32 / 1000.0),
        decode_ms: refresh.map_or(previous.decode_ms, |_| call.decode_us as f32 / 1000.0),
        mirror_ms: ms(mirror),
        entities: session.snapshot().len() as u32,
        snapshot_bytes: call.response_bytes,
    });
}

/// Whether the user is mid-drag on a transform handle.
///
/// Read off [`HandleSet`] rather than tracked separately: the handle set
/// already owns the `Idle → Hover → Drag` state machine, and a second
/// copy of "is the user dragging" would be one more thing to keep in
/// step. Absent in local mode tests and before the first frame, which
/// reads as "not dragging" — the safe answer, since the mirror is a
/// no-op without a session anyway.
fn drag_in_flight(resources: &Resources) -> bool {
    resources
        .get::<HandleSet>()
        .is_some_and(HandleSet::is_dragging)
}

#[cfg(test)]
mod tests;
