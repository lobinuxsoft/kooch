//! How many times one press is seen, at every ratio of send to tick.
//!
//! # The instrument, before the fix
//!
//! Reported as *"en Play un salto se dispara varias veces: una pulsación
//! se procesa dos o tres veces"* (#766), and **correct in the shipped
//! game** — which puts it on the editor → host path and nowhere else.
//!
//! This repo has three recorded cases of naming the cause by reading the
//! code and being wrong all three times. So this counts instead. The two
//! processes do not tick together and nothing synchronises them, so the
//! question is what one press looks like at each ratio:
//!
//! - **host faster than the editor** (the documented case, and #691 caps
//!   the editor near 20 FPS while the host runs free)
//! - **editor faster than the host**, which is what the issue guessed
//!
//! Each test names what it counted, so a failure says which of the two
//! it was rather than "input is broken".

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

/// 🔴 The documented case: the host ticks faster than the editor sends,
/// so one snapshot spans several host frames.
///
/// `just_pressed` is deliberately kept alive across them — that is the
/// #711 fix, and dropping it would lose the press entirely. So a
/// consumer reading the EDGE sees the same press on every frame until
/// the next snapshot supersedes it.
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

/// 🔴 The one that matters: a key **held** across several snapshots must
/// produce exactly one press, however many snapshots describe it.
///
/// This is the shape of the report — the finger never left the key.
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
