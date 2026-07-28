//! Tests for [`super`], against a real `RemoteServer` on loopback.
//!
//! The harness stands up an actual project loop in a thread, so these
//! exercise the same socket path the editor uses — including the wait
//! for the server's main thread, which is the cost #645 measures.

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

/// A socket name unique to this test.
///
/// Tests run in parallel in one process, so a shared name would have them
/// binding over each other — the local-socket equivalent of the port
/// scan this replaced, but solved instead of retried.
fn test_socket_name() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static N: AtomicU32 = AtomicU32::new(0);
    // The counter alone is not enough: it is per-module, so two test
    // modules in one binary both start at zero and collide on the same
    // name. The clock disambiguates without the modules having to know
    // about each other.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!(
        "ome_test_{}_{}_{}.sock",
        std::process::id(),
        nanos,
        N.fetch_add(1, Ordering::Relaxed)
    )
}

/// A project with one Transform-bearing entity at the origin, served
/// until `done` flips.
fn project(done: Arc<AtomicBool>) -> (String, std::thread::JoinHandle<()>) {
    let server = RemoteServer::start(&test_socket_name()).expect("bind");
    let socket = server.name().to_owned();
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
    (socket, thread)
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
fn connected(socket: &str, resources: &mut Resources) -> RemoteState {
    let mut state = RemoteState::new();
    state.session = Some(RemoteSession::attach(socket));
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
    let (socket, thread) = project(Arc::clone(&done));

    let mut editor = ecs();
    let state = connected(&socket, &mut editor);
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
    let (socket, thread) = project(Arc::clone(&done));

    let mut editor = ecs();
    let state = connected(&socket, &mut editor);
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

/// #645 — the pull's cost lands in the perf stats, with the
/// denominators that make it mean something.
#[test]
fn a_pull_records_what_it_cost() {
    let done = Arc::new(AtomicBool::new(false));
    let (socket, thread) = project(Arc::clone(&done));

    let mut editor = ecs();
    editor.insert(EditorPerfStats::default());
    let state = connected(&socket, &mut editor);
    editor.insert(state);
    tick(&mut editor, REFRESH_INTERVAL_IDLE + 1);

    let remote = editor
        .get::<EditorPerfStats>()
        .and_then(|s| s.remote)
        .expect("a connected session must report its cost");
    assert_eq!(remote.entities, 1, "the project serves one entity");
    assert!(
        remote.snapshot_bytes > 0,
        "a served snapshot cannot be zero bytes"
    );
    // The test server sleeps a millisecond between polls, so the
    // wait for its next turn is always measurable.
    assert!(remote.refresh_ms > 0.0, "got {}", remote.refresh_ms);
    assert!(remote.refresh_ms.is_finite());
    assert!(remote.transport_ms > 0.0, "got {}", remote.transport_ms);

    done.store(true, Ordering::Relaxed);
    thread.join().unwrap();
}

/// The split is the point: waiting on the project's frame boundary
/// must not be filed as parse cost, or the fix chosen from it is
/// the wrong one.
#[test]
fn the_wait_for_the_project_is_transport_not_decode() {
    let done = Arc::new(AtomicBool::new(false));
    let (socket, thread) = project(Arc::clone(&done));

    let mut editor = ecs();
    editor.insert(EditorPerfStats::default());
    let state = connected(&socket, &mut editor);
    editor.insert(state);
    tick(&mut editor, REFRESH_INTERVAL_IDLE + 1);

    let remote = editor
        .get::<EditorPerfStats>()
        .and_then(|s| s.remote)
        .unwrap();
    assert!(
        remote.transport_ms > remote.decode_ms,
        "a one-entity snapshot is trivial to parse; the cost is the wait \
         (transport {} ms, decode {} ms)",
        remote.transport_ms,
        remote.decode_ms
    );

    done.store(true, Ordering::Relaxed);
    thread.join().unwrap();
}

/// The cadence skips most frames, so the reading must persist
/// instead of blinking to zero twenty-nine frames out of thirty.
#[test]
fn the_sample_survives_the_frames_that_do_not_pull() {
    let done = Arc::new(AtomicBool::new(false));
    let (socket, thread) = project(Arc::clone(&done));

    let mut editor = ecs();
    editor.insert(EditorPerfStats::default());
    let state = connected(&socket, &mut editor);
    editor.insert(state);
    tick(&mut editor, REFRESH_INTERVAL_IDLE + 1);
    let after_pull = editor
        .get::<EditorPerfStats>()
        .and_then(|s| s.remote)
        .unwrap();

    // Well short of the next pull.
    tick(&mut editor, 2);
    let between = editor
        .get::<EditorPerfStats>()
        .and_then(|s| s.remote)
        .unwrap();
    assert_eq!(between, after_pull, "the reading was lost between pulls");

    done.store(true, Ordering::Relaxed);
    thread.join().unwrap();
}

/// Local mode has no pull at all, and reporting zeroes would read as
/// "measured, costs nothing". The HUD hides the section on `None`.
#[test]
fn local_mode_clears_the_remote_stats() {
    let mut editor = ecs();
    editor.insert(EditorPerfStats {
        remote: Some(RemoteSyncStats {
            refresh_ms: 7.0,
            ..Default::default()
        }),
        ..Default::default()
    });
    // A `RemoteState` with no session is exactly local mode.
    editor.insert(RemoteState::new());

    remote_sync_system(&mut editor);

    assert_eq!(
        editor.get::<EditorPerfStats>().and_then(|s| s.remote),
        None,
        "a stale remote reading outlived the session"
    );
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
