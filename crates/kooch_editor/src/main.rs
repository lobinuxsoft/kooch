//! Launcher hub: create, open or select projects, each a crate run with the editor embedded (`cargo
//! run`) or as a game (`cargo run -- --game`). The bootstrap is
//! [`kooch_editor_core::bootstrap`](kooch_editor_core::run_editor), shared with generated projects.

mod big_realloc;

#[global_allocator]
static ALLOCATOR: big_realloc::BigRealloc = big_realloc::BigRealloc;

fn main() {
    kooch_editor_core::run_editor();
}

#[cfg(test)]
mod profiler_required_tests;
