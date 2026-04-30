//! Hot-path streaming API for `OmeAccel` — `insert_chunk`,
//! `remove_chunk`, `refit_chunk`.
//!
//! Lives in its own module so the LBVH builder integration (which
//! lands in the slice-destination adapter commit) can grow without
//! pushing `state.rs` past the 400-LoC monolith line.
//!
//! Currently empty — the constructor + storage layout commit pinned
//! `OmeAccel::new`; this module wires up insert/remove/refit in the
//! follow-up commit alongside the slice-based builder adapter.
