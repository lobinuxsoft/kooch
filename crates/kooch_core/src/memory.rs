//! Handing freed heap back to the operating system (#1589).

use std::time::Duration;

/// Returns to the OS whatever the allocator is holding free, and how long that took.
///
/// 🔴 Freeing is not returning. glibc keeps freed heap inside the process for reuse, so RSS — what a
/// user reads as "memory" — does not fall when the engine releases a scene: measured, `heap_free`
/// went 3 → 53 MB across one scene change while RSS stayed put. Reproduced: 197 MB freed beneath
/// one live block left RSS at 193 MB; this brought it to 1 MB, in 4 ms. Call it after something was
/// released, never per frame: it walks the arena.
pub fn return_freed() -> Duration {
    let start = std::time::Instant::now();
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: `malloc_trim` takes a pad size and only operates on glibc's own arenas.
    unsafe {
        libc::malloc_trim(0);
    }
    start.elapsed()
}
