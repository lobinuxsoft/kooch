//! How many times one press is seen at every send-to-tick ratio (#766), counted rather than guessed
//! — for host-faster and editor-faster alike.

use super::*;

/// A snapshot holding exactly `keys`.
fn holding(keys: &[KeyCode]) -> InputSnapshot {
    InputSnapshot {
        keys: keys.to_vec(),
        ..Default::default()
    }
}

/// What a gameplay frame sees: the edge it would derive, and the events
/// it would drain.
struct Frame {
    edge: bool,
    presses: usize,
}

/// Runs `ticks` host frames against the snapshots already applied.
fn tick(backend: &mut RemoteInputBackend, ticks: usize) -> Vec<Frame> {
    (0..ticks)
        .map(|_| {
            backend.begin_frame();
            let presses = backend
                .poll()
                .iter()
                .filter(|event| matches!(event, InputEvent::KeyPressed(_)))
                .count();
            Frame {
                edge: backend.just_pressed(KeyCode::Space),
                presses,
            }
        })
        .collect()
}

/// 🔴 Host faster than the editor: one snapshot spans several frames, and `just_pressed` is kept
/// across them (#711), so the edge repeats until superseded.
#[test]
fn one_snapshot_three_ticks() {
    let mut backend = RemoteInputBackend::new();
    backend.apply(&holding(&[KeyCode::Space]));

    let frames = tick(&mut backend, 3);

    let presses: usize = frames.iter().map(|f| f.presses).sum();
    let edges = frames.iter().filter(|f| f.edge).count();
    assert_eq!(presses, 1, "the queue handed out the press more than once");
    assert_eq!(
        edges, 3,
        "the edge is live for every frame the snapshot spans — by design (#711), \
         and the reason a consumer must derive its own edge rather than read this",
    );
}

/// The issue's own guess: the editor sends faster than the host ticks,
/// so several snapshots land between two polls.
#[test]
fn three_snapshots_one_tick() {
    let mut backend = RemoteInputBackend::new();
    backend.apply(&holding(&[KeyCode::Space]));
    backend.apply(&holding(&[]));
    backend.apply(&holding(&[KeyCode::Space]));

    let frames = tick(&mut backend, 1);

    assert_eq!(
        frames[0].presses, 2,
        "two real presses arrived between two polls, and both are delivered — \
         this is the queue doing its job, not duplicating",
    );
}

/// 🔴 A key held across several snapshots produces exactly one press — the shape of the report.
#[test]
fn a_held_key_presses_once() {
    let mut backend = RemoteInputBackend::new();
    for _ in 0..5 {
        backend.apply(&holding(&[KeyCode::Space]));
    }

    let frames = tick(&mut backend, 1);

    assert_eq!(
        frames[0].presses, 1,
        "a key held across five snapshots was reported pressed more than once",
    );
}

/// And the consumer's own edge, derived the way a project derives it:
/// `held && !held_last_frame`. One press, one edge, whatever the ratio.
#[test]
fn a_derived_edge_fires_once() {
    let mut backend = RemoteInputBackend::new();
    let mut was_held = false;
    let mut fired = 0usize;

    // Editor sends once; host ticks three times. Then the key is
    // released, and sent once.
    backend.apply(&holding(&[KeyCode::Space]));
    for _ in 0..3 {
        let held = backend.is_pressed(KeyCode::Space);
        fired += usize::from(held && !was_held);
        was_held = held;
    }
    backend.apply(&holding(&[]));
    for _ in 0..3 {
        let held = backend.is_pressed(KeyCode::Space);
        fired += usize::from(held && !was_held);
        was_held = held;
    }

    assert_eq!(fired, 1, "one press produced {fired} edges");
}
