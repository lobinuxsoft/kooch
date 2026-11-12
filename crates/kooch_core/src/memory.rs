//! Handing freed heap back to the operating system (#1589), and keeping big blocks out of it (#1891).

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

/// Serves every allocation of 1 MiB or more from its own mapping, returned to the OS when freed.
///
/// 🔴 glibc raises this threshold each time a mapped block is freed, so after one 12 MB video frame
/// every smaller big block lands in the main heap and fragments it: measured +104 MB in 3 min, flat
/// once fixed. Setting it turns the raising off for the whole process, C libraries and plugins too.
pub fn map_big_blocks() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: `mallopt` only changes glibc's allocator settings; a refused value returns 0.
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 1 << 20);
    }
}
