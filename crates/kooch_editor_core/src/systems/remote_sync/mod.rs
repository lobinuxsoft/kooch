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

use kooch_core::resource::Resources;
use kooch_gizmos_handles::HandleSet;

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
    /// Asks for a pull on the next frame instead of at the next tick of
    /// the cadence.
    ///
    /// 🔴 An edit the editor *just sent* is not "outside change the
    /// project might have made" — it is a change the editor is waiting
    /// to see. Without this it waits out the poll: up to half a second
    /// where the world has already gone back and the screen has not,
    /// which reads exactly like a Ctrl+Z that was ignored. It is why the
    /// chord "needed two presses" — the second one was the first one
    /// arriving, plus a second step undone.
    pub(crate) fn invalidate(&mut self) {
        self.last_pull = None;
    }

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
    profiling::scope!("remote sync");
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
        pending_selection,
        // The connecting banner's copy of the build log. Nothing here
        // reads it; it is kept and cleared where the session is made.
        connect_output: _,
        // Owned by the input sender, which runs after this.
        last_input_was_idle: _,
    } = state;
    let Some(session) = session.as_mut() else {
        // Local mode costs nothing remote, and saying so is not the
        // same as saying zero — the HUD hides the section entirely.
        if let Some(stats) = resources.get_mut::<EditorPerfStats>() {
            stats.remote = None;
            stats.host = None;
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
        // 🔴 While playing the editor cannot edit, so it does not need
        // the world — it needs what moved (#1012). `refresh` makes the
        // host reflect every field of every component of every entity
        // into strings before it diffs: 38.9 ms of a 46 ms frame on
        // `dense.scene`, waited for on this thread. The cheap pull reads
        // one column and compares floats.
        //
        // `None` is the host declining: its entity set changed, so a
        // transform diff would describe a world this does not have. The
        // full pull runs on that frame and the cheap one resumes.
        let moved = if *playing {
            profiling::scope!("remote: pull moved");
            session.refresh_moved()
        } else {
            None
        };
        match moved {
            Some(moved) => {
                refresh = Some(started.elapsed());
                sync.last_pull = Some(Instant::now());
                profiling::scope!("remote: apply moved");
                mirror.apply_moved(&moved, resources);
                record_stats(resources, session, refresh, Duration::ZERO);
                return;
            }
            None => {
                profiling::scope!("remote: pull");
                session.refresh();
            }
        }
        refresh = Some(started.elapsed());
        sync.last_pull = Some(Instant::now());
    }

    // Applying a snapshot walks every entity — about 7.5 ms on 610 of
    // them, more than the pull costs now that the pull is a diff. When
    // the project reported nothing new there is nothing to walk for, so
    // the mirror keeps what it has and the frame keeps the time (#691).
    //
    // `just_connected` is exempt: the handshake's snapshot has never
    // been applied, and the mirror on that frame is empty.
    let applying = Instant::now();
    let mirror_time = if just_connected || session.changed_last_refresh() {
        profiling::scope!("remote: apply");
        mirror.apply(session.snapshot(), resources);
        // Now that the snapshot has landed, whatever the last creation
        // made has a mirror handle to select.
        take_selection(resources, mirror, pending_selection);
        applying.elapsed()
    } else {
        Duration::ZERO
    };
    record_stats(resources, session, refresh, mirror_time);
}

/// Selects what the project just created, once the mirror can name it.
///
/// Nothing happens until *every* id resolves: a paste of three entities
/// selects three, not the first one to arrive. Ids that never turn up
/// are dropped with the rest — a creation the project refused should not
/// leave the editor waiting for it forever.
fn take_selection(
    resources: &mut Resources,
    mirror: &crate::remote_mirror::RemoteMirror,
    pending: &mut Vec<kooch_remote::protocol::EntityId>,
) {
    if pending.is_empty() {
        return;
    }
    let entities: Vec<_> = pending
        .iter()
        .filter_map(|id| mirror.local_of(*id))
        .collect();
    if entities.len() != pending.len() {
        return;
    }
    pending.clear();
    if let Some(overlay) = resources.get_mut::<crate::state::EditorOverlay>() {
        overlay.selected_entities = entities;
        // The Inspector shows one subject: selecting an entity means the
        // asset selection is over, the same arbitration the UI does when
        // the click comes from a panel.
        overlay.selected_asset = None;
        overlay.last_clicked_index = None;
    }
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
    // Held across frames that skip the pull, like the numbers above: a
    // section that blanks on twenty-nine frames out of thirty is a
    // section nobody can read.
    if let Some(host) = session.host_metrics() {
        stats.host = Some(host);
    }
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
