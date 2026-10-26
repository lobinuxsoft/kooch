//! Launcher hub: create, open or select projects, each a crate run with the editor embedded (`cargo
//! run`) or as a game (`cargo run -- --game`). The bootstrap is
//! [`kooch_editor_core::bootstrap`](kooch_editor_core::run_editor), shared with generated projects.

// Temporary, for #1589: every allocation is attributed to its backtrace.
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOCATOR: dhat::Alloc = dhat::Alloc;

fn main() {
    // Written when this drops, which is when the event loop returns: closing the window, not a kill.
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::builder()
        .file_name("/tmp/kooch-dhat-heap.json")
        .build();
    kooch_editor_core::run_editor();
}

#[cfg(test)]
mod profiler_required_tests;
