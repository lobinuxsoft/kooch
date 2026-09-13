//! The physical page pool and its table (#866).

use super::PageConfig;

/// Words per table entry — the slot, its age, its place in the compacted page list, and the content
/// stamp the cache runs on. Mirrors `PAGE_CELL` in `page_table.wgsl`, which is where the reason
/// lives.
pub const PAGE_CELL: u32 = 6;

/// `KOOCH_SHADOW_POOL_PAGES`, read once.
pub fn pages_from_environment() -> u32 {
    static PAGES: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *PAGES.get_or_init(|| {
        std::env::var("KOOCH_SHADOW_POOL_PAGES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_PAGES)
            .clamp(PAGES_RANGE.0, PAGES_RANGE.1)
    })
}

/// What the pool is sized to when nobody says otherwise.
pub const DEFAULT_PAGES: u32 = 2048;

/// What the pool may be sized to.
pub const PAGES_RANGE: (u32, u32) = (4, 8192);

/// Views a single pool may be sliced between.
pub const VIEWS_RANGE: (u32, u32) = (1, 8);

/// How the physical pool is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolConfig {
    /// Physical pages the budget asks for, across every view. Epic's
    /// default is 4096; see [`POOL_PAGES`](super::POOL_PAGES).
    pub pages: u32,
    /// Cameras sharing it.
    pub views: u32,
    /// Pages a layer may hold across, from the device's texture limit (#1016).
    pub row_cap: u32,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            pages: pages_from_environment(),
            views: 1,
            // No device has spoken yet; `fit_atlas` is what narrows it.
            row_cap: u32::MAX,
        }
    }
}

impl PoolConfig {
    /// Cameras the pool is really sliced between — `views`, clamped.

    /// The same pool, told how wide a layer the device can hold.
    pub fn fit_atlas(mut self, max_side: u32, page: u32) -> Self {
        let device = (max_side / page.max(1)).max(1);
        // 🔴 An override that can only make the layer SMALLER, and it exists because the multi-layer
        // path is otherwise unreachable where anyone can look at it.
        self.row_cap = row_cap_from_environment().unwrap_or(device).min(device);
        self
    }

    /// The same pool, sliced between `views` cameras.
    pub fn with_views(self, views: u32) -> Self {
        Self {
            views: views.clamp(VIEWS_RANGE.0, VIEWS_RANGE.1),
            ..self
        }
    }

    fn clamped(&self) -> (u32, u32) {
        (
            self.pages.clamp(PAGES_RANGE.0, PAGES_RANGE.1),
            self.views.clamp(VIEWS_RANGE.0, VIEWS_RANGE.1),
        )
    }

    /// Pages across one view's layer, in both axes.
    pub fn per_row(&self) -> u32 {
        let (pages, views) = self.clamped();
        let square = ((pages.div_ceil(views)) as f64).sqrt().ceil().max(1.0) as u32;
        square.min(self.row_cap.max(1))
    }

    /// Pages ONE LAYER holds. `page_place` reads a slot as
    /// `slot % slice` inside the layer and `slot / slice` for the layer
    /// itself, so this is the number the shaders address by.
    pub fn slice(&self) -> u32 {
        self.per_row().pow(2)
    }

    /// Layers one view needs to hold its share of the budget (#1016).
    pub fn layers_per_view(&self) -> u32 {
        let (pages, views) = self.clamped();
        pages.div_ceil(views).div_ceil(self.slice().max(1)).max(1)
    }

    /// Pages one view owns, across all of its layers — the capacity
    /// every per-view counter is read against.
    pub fn slots(&self) -> u32 {
        self.slice() * self.layers_per_view()
    }

    /// The atlas's array depth. NOT the view count — see [`Self::views`].
    pub fn layers(&self) -> u32 {
        self.clamped().1 * self.layers_per_view()
    }

    /// Cameras sharing the pool, clamped to what the layout allows.
    pub fn view_count(&self) -> u32 {
        self.clamped().1
    }

    /// Pages the pool really holds, which is a view's slots times the views.
    pub fn total(&self) -> u32 {
        self.slots() * self.clamped().1
    }

    /// Where a view's slice starts, in global slot numbers.
    pub fn base(&self, view: u32) -> u32 {
        view.min(self.clamped().1 - 1) * self.slots()
    }

    /// What the atlas costs, at `Depth32Float`.
    pub fn atlas_bytes(&self, config: PageConfig) -> u64 {
        self.total() as u64 * config.page_bytes()
    }
}

/// How long a page outlives the frame that asked for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolLife {
    /// This frame's index, counting up for the process's lifetime.
    pub frame: u32,
    /// Frames a page may go unrequested before it is evicted.
    pub max_age: u32,
    /// Set when the pool was just rebuilt, which evicts everything: a
    /// slot recorded against the old atlas names a different page in the
    /// new one.
    pub rebuilt: bool,
}

impl Default for PoolLife {
    fn default() -> Self {
        Self {
            frame: 0,
            max_age: age_from_environment(),
            rebuilt: false,
        }
    }
}

impl PoolLife {
    /// The uniform's `life` field.
    pub fn words(&self) -> [u32; 4] {
        [self.frame, self.max_age, u32::from(self.rebuilt), 0]
    }
}

/// Frames a page survives unrequested when nobody says otherwise.
pub const DEFAULT_MAX_AGE: u32 = 60;

/// How long a page survives unrequested, when nobody says otherwise.
pub const DEFAULT_AGE_SECONDS: f32 = 1.0;

/// `KOOCH_SHADOW_ROW_CAP`, read once — pages a layer may hold across.
pub fn row_cap_from_environment() -> Option<u32> {
    static CAP: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *CAP.get_or_init(|| {
        let cap = std::env::var("KOOCH_SHADOW_ROW_CAP")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|c| *c > 0);
        if let Some(cap) = cap {
            tracing::info!(
                target: "kooch_render::shadow",
                cap,
                "KOOCH_SHADOW_ROW_CAP: the atlas layer is narrowed on purpose",
            );
        }
        cap
    })
}

/// `KOOCH_SHADOW_PAGE_SECONDS`, read once.
pub fn age_seconds() -> f32 {
    static SECONDS: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *SECONDS.get_or_init(|| {
        std::env::var("KOOCH_SHADOW_PAGE_SECONDS")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|s| *s > 0.0)
            .unwrap_or(DEFAULT_AGE_SECONDS)
            .min(16.0)
    })
}

/// `KOOCH_SHADOW_PAGE_AGE`, read once. See [`PoolLife`] for why the
/// default is long rather than short.
pub fn age_from_environment() -> u32 {
    static AGE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *AGE.get_or_init(|| {
        std::env::var("KOOCH_SHADOW_PAGE_AGE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_MAX_AGE)
            .min(1024)
    })
}

/// The page table and the allocator's state, rebuilt when the pool is resized.
pub struct PagePool {
    /// The FLAT page table: `PAGE_CELL` words per VIRTUAL page, indexed
    /// by the page id itself. See `page_table.wgsl` for the entry
    /// layout and for why the hash this replaced is gone.
    slots: wgpu::Buffer,

    /// `[high, free_count, free_slots...]` per view — see `alloc_base`
    /// in the shader.
    alloc: wgpu::Buffer,
    config: PoolConfig,
    /// Virtual pages the table holds entries for, across every view.
    entries: u32,
}

impl PagePool {
    pub fn new(device: &wgpu::Device, config: PoolConfig) -> Self {
        // 🔴 Said out loud because a BUILD has no Shadow pages panel, and the layer split only ever
        // happens in a build: the pool is divided between views, so the editor's two cameras never
        // cross a layer boundary.
        tracing::info!(
            target: "kooch_render::shadow",
            pages = config.pages,
            views = config.view_count(),
            per_row = config.per_row(),
            per_layer = config.slice(),
            layers_per_view = config.layers_per_view(),
            layers = config.layers(),
            slots_per_view = config.slots(),
            total = config.total(),
            "the shadow page atlas is laid out",
        );
        Self {
            slots: table_buffer(device, "shadow_page_cells", PAGE_CELL),
            alloc: table_buffer(
                device,
                "shadow_page_alloc",
                config.view_count() * (config.slots() + 2),
            ),
            config,
            entries: 1,
        }
    }

    pub fn config(&self) -> PoolConfig {
        self.config
    }

    /// Virtual pages the table holds entries for.
    pub fn entries(&self) -> u32 {
        self.entries
    }

    /// Resizes if the pool changed, and reports whether it did.
    pub fn resize(&mut self, device: &wgpu::Device, config: PoolConfig) -> bool {
        if config == self.config {
            return false;
        }
        let entries = self.entries;
        *self = Self::new(device, config);
        self.ensure_entries(device, entries);
        true
    }

    /// Grows the table to `entries` virtual pages, and reports whether the buffers were replaced —
    /// in which case every entry is gone and the caller flags a rebuild.
    pub fn ensure_entries(&mut self, device: &wgpu::Device, entries: u32) -> bool {
        if entries <= self.entries {
            return false;
        }
        self.slots = table_buffer(device, "shadow_page_cells", entries * PAGE_CELL);
        self.alloc = table_buffer(
            device,
            "shadow_page_alloc",
            self.config.view_count() * (self.config.slots() + 2),
        );
        self.entries = entries;
        true
    }

    pub fn slots(&self) -> &wgpu::Buffer {
        &self.slots
    }

    pub fn alloc(&self) -> &wgpu::Buffer {
        &self.alloc
    }

    /// Empties the WHOLE table and resets the allocator, every view's entries included.
    pub fn clear(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.clear_buffer(&self.slots, 0, None);
        encoder.clear_buffer(&self.alloc, 0, None);
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
    pub claims: u32,
    /// Requests that found their page ALREADY RESIDENT, so nothing was allocated and nothing has to
    /// be rasterised again.
    pub reused: u32,
    /// Resident pages this view carried over — requested recently enough
    /// not to be evicted.
    pub alive: u32,
    /// Pages freed this frame for going unrequested past `max_age`.
    pub evicted: u32,
    /// Slots the free list could not hold, which is a double free.
    /// 🔴 Always zero, or the allocator is wrong.
    pub leaked: u32,
    /// Claims past the end of the pool. Non-zero means pages went unshadowed this frame; Epic's own
    /// overflow shows up as checkerboard corruption, so the counter exists to name it before anyone
    /// has to recognise it by sight.
    pub overflow: u32,
    /// Marked pages the seating plan turned away: their rank was past the cutoff the slice's budget
    /// reaches (#942).
    pub denied: u32,
    /// Residents evicted by pressure rather than by age: the plan did not fund their rank this
    /// frame, so their seat went to a higher rank. Persistent churn here with a still camera means
    /// the demand is oscillating around the cutoff.
    pub preempted: u32,
    /// The rank the plan funded down to. `RANKS` (32) when everything
    /// fit; the sun's clipmap occupies ranks 0..17, the local chains
    /// the ranks after it, coarsest first.
    pub cutoff: u32,
    /// Levels of resolution the LOCAL lights are marked coarser than
    /// the screen asked for (#943). Zero when the demand fits; each
    /// step is a quarter of the pages. Locals pay before the sun.
    pub bias_local: u32,
    /// Levels the SUN is marked coarser. Paid only once the locals
    /// have given up `LOCAL_BIAS_MAX` levels and the demand still does
    /// not fit — the shadow everyone sees degrades last.
    pub bias_sun: u32,
    /// Slots the bump allocator has ever handed out, which never goes down: a freed slot returns to
    /// the free list, not to the bump.
    pub high: u32,
    /// Slots on this view's free list after the frame was seated.
    pub free: u32,
    /// Pages the frame's screen asked for — the total `plan_view` budgeted against.
    pub demand: u32,
    /// Slots taken off the free list this frame.
    pub popped: u32,
    /// Slots taken from the bump — never handed out before.
    pub bumped: u32,
    /// Slots given back to the free list this frame.
    pub pushed: u32,
    /// Pops that found the list empty.
    pub empty: u32,
    /// Physical pages THIS VIEW owns, so the two numbers above are readable without knowing how the
    /// build was configured.
    pub capacity: u32,
}

impl PoolCounts {
    /// Slots the pool is holding after this frame: what survived the ageing, plus what was
    /// allocated on top of it.
    pub fn allocated(&self) -> u32 {
        // `alive` is counted by the ageing, BEFORE the seat passes run;
        // what pressure then preempted is no longer held.
        (self.alive + self.claims)
            .saturating_sub(self.preempted)
            .min(self.capacity)
    }

    /// Pages the frame marked and could not spend a slot on, because
    /// nothing draws them yet.
    pub fn unspent(&self, resident: u32) -> u32 {
        resident.saturating_sub(self.claims + self.reused)
    }

    /// Requests that reached the pool at all — a reuse or an allocation.
    pub fn requests(&self) -> u32 {
        self.claims + self.reused
    }

    /// How much of this frame's work the pool answered from what it already had, as a percentage.
    pub fn hit_rate(&self) -> f32 {
        let requests = self.requests();
        if requests == 0 {
            return 0.0;
        }
        self.reused as f32 / requests as f32 * 100.0
    }

    /// Whether the allocator's ledger closes: every slot is either held by a resident or sitting on
    /// the free list.
    pub fn balanced(&self) -> bool {
        self.high < self.capacity || self.allocated() + self.free == self.capacity
    }

    /// How full the pool ran, as a percentage.
    pub fn load(&self) -> f32 {
        if self.capacity == 0 {
            return 0.0;
        }
        self.allocated() as f32 / self.capacity as f32 * 100.0
    }
}

#[cfg(test)]
mod atlas_layer_tests;
