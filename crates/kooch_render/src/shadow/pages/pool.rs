//! The physical page pool and its table (#866).
//!
//! Marking answers *which pages does this frame need*. This answers
//! *where does each of them live*, and the two run in the **same
//! dispatch**: `mark_bit` already reports the thread that flipped a
//! page's bit from 0 to 1, so claiming a physical slot there is one
//! `atomicAdd` on a rare branch. No second pass, and nothing walks the
//! virtual space.
//!
//! # What the table is, and why the flat answer is dead
//!
//! The arithmetic is in `page_table.wgsl` next to the code that depends
//! on it, in full. The short version: 101 lights and a sun address
//! **28 409 856** virtual pages. One bit each is 3.4 MiB and that is the
//! mark bitmap; one `u32` each is **108 MiB — 42 % of the 256 MiB pool
//! it would index**. So the table is sized to what is RESIDENT rather
//! than to what is addressable: `2 x pages` entries of open addressing,
//! **64 KiB** at Epic's 4096-page pool.
//!
//! # What is deliberately not here yet
//!
//! 🔴 The **atlas texture**. A pool of 4096 pages at `Depth32Float` is
//! 256 MiB, and nothing writes or reads it until the depth raster
//! lands. Allocating it now would be a quarter of a gigabyte spent to
//! make a diagram look finished — the memory version of the
//! backend-without-a-panel this project already refuses. The layout it
//! will have is fixed and testable regardless: `page_origin` in the
//! shader, [`PoolConfig::per_row`] here.

use super::PageConfig;

/// `KOOCH_SHADOW_POOL_PAGES`, read once.
///
/// An environment variable and **not** a `.rendersettings` field, on
/// purpose: #477 asks that nothing on the shadow side grow a public
/// setting before the pool's shape is decided, and a knob that sizes an
/// atlas nobody allocates yet would promise memory nothing spends. It
/// becomes a setting when the raster does.
pub fn pages_from_environment() -> u32 {
    static PAGES: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *PAGES.get_or_init(|| {
        std::env::var("KOOCH_SHADOW_POOL_PAGES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(super::POOL_PAGES)
            .clamp(PAGES_RANGE.0, PAGES_RANGE.1)
    })
}

/// What the pool may be sized to.
///
/// The floor is absurdly small on purpose: overflow is the one failure
/// mode nobody recognises by sight — Epic's own shows up as
/// checkerboard corruption — so a test has to be able to fill the pool
/// deliberately. The ceiling is where Epic's tuning notes say the pool
/// starts thrashing rather than helping.
pub const PAGES_RANGE: (u32, u32) = (4, 8192);

/// How the physical pool is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolConfig {
    /// Physical pages. Epic's default is 4096; see
    /// [`POOL_PAGES`](super::POOL_PAGES).
    pub pages: u32,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            pages: pages_from_environment(),
        }
    }
}

impl PoolConfig {
    /// Entries in the hash table.
    ///
    /// Twice the pool, rounded up to a power of two, so the load factor
    /// never passes 0.5 and the mask is an `and` rather than a modulo.
    /// The expected probe count at that load is under two.
    pub fn entries(&self) -> u32 {
        self.pages
            .clamp(PAGES_RANGE.0, PAGES_RANGE.1)
            .next_power_of_two()
            * 2
    }

    /// Pages across the atlas.
    ///
    /// Square-ish, because a long strip wastes the second dimension of
    /// every texture limit there is: 8192 pages in a row is past
    /// `max_texture_dimension_2d` the moment a page is more than 8
    /// texels.
    pub fn per_row(&self) -> u32 {
        (self.pages as f64).sqrt().ceil() as u32
    }

    /// What the atlas will cost once it exists, at `Depth32Float`.
    pub fn atlas_bytes(&self, config: PageConfig) -> u64 {
        self.pages as u64 * config.page_bytes()
    }

    /// What the table costs, which is the number the flat answer lost
    /// to: keys and slots, one `u32` each.
    pub fn table_bytes(&self) -> u64 {
        self.entries() as u64 * 8
    }
}

/// The page table's two buffers, rebuilt when the pool is resized.
///
/// Cleared every frame rather than reset by a pass: `PAGE_EMPTY` is 0,
/// so `clear_buffer` IS the reset. That is the whole reason keys are
/// stored as `page + 1`.
///
/// ⚠️ Clearing every frame also means nothing is cached across frames,
/// which is the optimisation real VSM exists for — a static shadow
/// re-rasterised every frame is the cost caching removes. Correct
/// first; the cache needs an eviction policy and an invalidation rule,
/// and neither can be designed before anything renders.
pub struct PagePool {
    keys: wgpu::Buffer,
    slots: wgpu::Buffer,
    config: PoolConfig,
}

impl PagePool {
    pub fn new(device: &wgpu::Device, config: PoolConfig) -> Self {
        Self {
            keys: table_buffer(device, "shadow_page_keys", config.entries()),
            slots: table_buffer(device, "shadow_page_slots", config.entries()),
            config,
        }
    }

    pub fn config(&self) -> PoolConfig {
        self.config
    }

    /// Resizes if the pool changed, and reports whether it did.
    pub fn resize(&mut self, device: &wgpu::Device, config: PoolConfig) -> bool {
        if config == self.config {
            return false;
        }
        *self = Self::new(device, config);
        true
    }

    pub fn keys(&self) -> &wgpu::Buffer {
        &self.keys
    }

    pub fn slots(&self) -> &wgpu::Buffer {
        &self.slots
    }

    /// Empties the table. Only the keys need it — a slot is never read
    /// without its key matching first.
    pub fn clear(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.clear_buffer(&self.keys, 0, None);
    }
}

fn table_buffer(device: &wgpu::Device, label: &str, entries: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: entries as u64 * 4,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

/// What the allocator did, alongside what marking found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolCounts {
    /// Pages that asked for a physical slot.
    ///
    /// 🔴 Equal to `MarkCounts::resident` when the allocator is sound —
    /// both count the 0→1 transitions, by two different mechanisms. A
    /// disagreement means one of them is broken, and that cross-check is
    /// why this is reported rather than derived.
    pub claims: u32,
    /// Claims past the end of the pool. Non-zero means pages went
    /// unshadowed this frame; Epic's own overflow shows up as
    /// checkerboard corruption, so the counter exists to name it before
    /// anyone has to recognise it by sight.
    pub overflow: u32,
    /// Inserts that walked `PAGE_PROBES` slots without finding room.
    ///
    /// ⚠️ Distinct from `overflow`: the pool had space and the TABLE
    /// did not. At a load factor of 0.5 this should be zero, and any
    /// other number is a statement about the hash rather than about the
    /// scene.
    pub probes: u32,
    /// Physical pages the pool holds, so the two numbers above are
    /// readable without knowing how the build was configured.
    pub capacity: u32,
}

impl PoolCounts {
    /// Slots actually handed out, which is `claims` capped by the pool.
    pub fn allocated(&self) -> u32 {
        self.claims.min(self.capacity)
    }

    /// How full the pool ran, as a percentage.
    pub fn load(&self) -> f32 {
        if self.capacity == 0 {
            return 0.0;
        }
        self.allocated() as f32 / self.capacity as f32 * 100.0
    }
}
