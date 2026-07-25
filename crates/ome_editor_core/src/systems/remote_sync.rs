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
use ome_gizmos_handles::HandleSet;

use crate::remote_session::{ConnectionState, RemoteState};

/// Frames between snapshot pulls while the project sits paused.
///
/// The project owns the world and may mutate it behind the editor's
/// back, so the mirror is a poll, not a subscription. Twice a second at
/// 60 fps keeps the editor responsive to outside change without
/// spending a synchronous HTTP round-trip per frame.
const REFRESH_INTERVAL_IDLE: u32 = 30;

/// Frames between snapshot pulls while the project is playing.
///
/// Gameplay moves things every tick, so a paused-mode cadence would
/// render as a slideshow. The cost is one round-trip per frame over
/// loopback; the mirror diffs in place, so a pull that changes nothing
/// structural is just field writes.
const REFRESH_INTERVAL_PLAYING: u32 = 1;

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
    let RemoteState {
        session,
        mirror,
        playing,
    } = state;
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
        // Hold the cadence at zero so the frame after release does not
        // immediately apply the snapshot we just skipped.
        sync.frames = 0;
        return;
    }

    if !just_connected {
        let interval = if *playing {
            REFRESH_INTERVAL_PLAYING
        } else {
            REFRESH_INTERVAL_IDLE
        };
        sync.frames += 1;
        if sync.frames < interval {
            return;
        }
        sync.frames = 0;
        session.refresh();
    }

    mirror.apply(session.snapshot(), resources);
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
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use glam::Vec3;
    use ome_core::resource::Resources;
    use ome_ecs::allocator::EntityAllocator;
    use ome_ecs::archetype_registry::ArchetypeRegistry;
    use ome_ecs::commands::Commands;
    use ome_ecs::component::{ComponentNames, ComponentRegistry};
    use ome_ecs::dynamic_components::DynamicComponents;
    use ome_ecs::entity::Entity;
    use ome_ecs::name::Name;
    use ome_ecs::query::AccessTracker;
    use ome_ecs::transform::Transform;
    use ome_gizmos_handles::{DragModifiers, HandleMode, Ray, SnapSettings};
    use ome_remote::handlers::handle;
    use ome_remote::server::RemoteServer;

    use super::*;
    use crate::remote_session::RemoteSession;

    fn ecs() -> Resources {
        let mut r = Resources::new();
        r.insert(EntityAllocator::new());
        r.insert(ComponentRegistry::new());
        r.insert(ArchetypeRegistry::new());
        r.insert(AccessTracker::new());
        r.insert(Commands::new());
        r.insert(DynamicComponents::new());
        r.insert(ComponentNames::new());
        {
            let reg = r.get_mut::<ComponentRegistry>().unwrap();
            reg.register_cpu_reflected::<Name>();
            reg.register_cpu_reflected::<Transform>();
        }
        r
    }

    /// A `HandleSet` driven into a drag: the centre scale cube sits at
    /// the origin, so a ray straight at it picks with the button down.
    fn dragging_handles() -> HandleSet {
        let mut handles = HandleSet::default();
        handles.set_mode(HandleMode::Scale);
        handles.set_origin(Vec3::ZERO);
        handles.update(
            Some(Ray::new(Vec3::new(0.0, 0.0, 5.0), -Vec3::Z)),
            true,
            true,
            DragModifiers::default(),
            SnapSettings::default(),
        );
        assert!(handles.is_dragging(), "test setup did not start a drag");
        handles
    }

    /// A project with one Transform-bearing entity at the origin, served
    /// until `done` flips.
    fn project(done: Arc<AtomicBool>) -> (u16, std::thread::JoinHandle<()>) {
        let server = (0..8)
            .find_map(|i| RemoteServer::start(17800 + i).ok())
            .expect("bind");
        let port = server.port();
        let thread = std::thread::spawn(move || {
            let mut res = ecs();
            let entity = {
                let mut commands = res.remove::<Commands>().unwrap();
                let entity = commands.spawn(&mut res).id();
                commands.apply(&mut res);
                res.insert(commands);
                entity
            };
            insert_transform(&mut res, entity);
            while !done.load(Ordering::Relaxed) {
                for item in server.take_pending() {
                    let resp = handle(&item.request, &mut res);
                    let _ = item.reply.send(resp);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        });
        (port, thread)
    }

    fn insert_transform(resources: &mut Resources, entity: Entity) {
        use std::any::TypeId;
        if let Some(reg) = resources.get_mut::<ComponentRegistry>() {
            reg.insert_default_reflected(&TypeId::of::<Transform>(), entity);
        }
        let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() else {
            return;
        };
        let empty = archetypes.get_or_create(Default::default());
        archetypes.register_entity(entity, empty);
        let next = archetypes.archetype_after_add::<Transform>(empty);
        archetypes.register_entity(entity, next);
    }

    /// Connects an editor-side `RemoteState` and mirrors once.
    fn connected(port: u16, resources: &mut Resources) -> RemoteState {
        let mut state = RemoteState::new();
        state.session = Some(RemoteSession::attach(port));
        for _ in 0..200 {
            if state.session.as_mut().unwrap().poll_ready() == ConnectionState::Connected {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(state.is_connected(), "did not connect");
        let snapshot = state.session.as_ref().unwrap().snapshot().to_vec();
        state.mirror.apply(&snapshot, resources);
        state
    }

    /// The mirrored entity's local position.
    fn mirrored_position(resources: &Resources, state: &RemoteState) -> Vec3 {
        let local = state
            .session
            .as_ref()
            .unwrap()
            .snapshot()
            .first()
            .and_then(|s| state.mirror.local_of(s.id))
            .expect("nothing mirrored");
        resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<Transform>())
            .and_then(|s| s.get(local))
            .map(|t| t.position)
            .expect("mirror has no Transform")
    }

    /// Moves the mirrored entity locally, the way a gizmo drag does.
    fn drag_it_to(resources: &mut Resources, state: &RemoteState, position: Vec3) {
        let local = state
            .session
            .as_ref()
            .unwrap()
            .snapshot()
            .first()
            .and_then(|s| state.mirror.local_of(s.id))
            .expect("nothing mirrored");
        if let Some(reg) = resources.get_mut::<ComponentRegistry>()
            && let Some(storage) = reg.get_cpu_mut::<Transform>()
            && let Some(t) = storage.get_mut(local)
        {
            t.position = position;
        }
    }

    /// Runs the system `frames` times, enough to cross the idle cadence.
    fn tick(resources: &mut Resources, frames: u32) {
        for _ in 0..frames {
            remote_sync_system(resources);
        }
    }

    /// The bug: a drag lasts many frames, and the refresh landed on top
    /// of it every 30th one — snapping the entity back to the value the
    /// project still holds, because the edit only ships on release.
    #[test]
    fn a_drag_in_flight_survives_the_refresh_cadence() {
        let done = Arc::new(AtomicBool::new(false));
        let (port, thread) = project(Arc::clone(&done));

        let mut editor = ecs();
        let state = connected(port, &mut editor);
        let dragged_to = Vec3::new(4.0, 5.0, 6.0);
        drag_it_to(&mut editor, &state, dragged_to);

        editor.insert(state);
        editor.insert(dragging_handles());
        tick(&mut editor, REFRESH_INTERVAL_IDLE * 3);

        let state = editor.remove::<RemoteState>().unwrap();
        assert_eq!(
            mirrored_position(&editor, &state),
            dragged_to,
            "the mirror clobbered the drag in progress"
        );

        done.store(true, Ordering::Relaxed);
        thread.join().unwrap();
    }

    /// And once the drag ends the poll resumes: the project is still the
    /// source of truth, so a local value the project never received goes
    /// away. (In the editor it does not, because releasing the handle
    /// dispatches the edit first — that path is covered in `remote_edit`.)
    #[test]
    fn the_refresh_resumes_once_the_drag_ends() {
        let done = Arc::new(AtomicBool::new(false));
        let (port, thread) = project(Arc::clone(&done));

        let mut editor = ecs();
        let state = connected(port, &mut editor);
        drag_it_to(&mut editor, &state, Vec3::new(4.0, 5.0, 6.0));

        editor.insert(state);
        // No HandleSet at all — the same as a released handle.
        tick(&mut editor, REFRESH_INTERVAL_IDLE + 1);

        let state = editor.remove::<RemoteState>().unwrap();
        assert_eq!(
            mirrored_position(&editor, &state),
            Vec3::ZERO,
            "the mirror stopped tracking the project"
        );

        done.store(true, Ordering::Relaxed);
        thread.join().unwrap();
    }

    /// An idle handle set is not a drag — hovering must not freeze the
    /// mirror, or a cursor parked over a gizmo stops the editor updating.
    #[test]
    fn an_idle_handle_set_does_not_block_the_refresh() {
        let mut resources = Resources::new();
        assert!(!drag_in_flight(&resources), "no handles is not a drag");

        resources.insert(HandleSet::default());
        assert!(
            !drag_in_flight(&resources),
            "an idle handle set is not a drag"
        );

        resources.insert(dragging_handles());
        assert!(drag_in_flight(&resources));
    }
}
