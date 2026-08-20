// page_table.wgsl — the virtual page id and where it lands (#866).
//
// CONCATENATED into every pass that touches the page table: the marking
// pass that WRITES it and, later, the shading pass that READS it. This
// file holds what the two must agree on and nothing else.
//
// # Why a hash, and why the obvious answer is dead
//
// With 128-texel pages over a 16384 virtual map, a mip chain per cube
// face and a 17-level clipmap, one light addresses 278 528 pages. A
// hundred lights and a sun make the virtual space **28 409 856 pages**.
//
// - The MARK bitmap is one bit each: 3.4 MiB. Affordable, and that is
//   why marking was built first.
// - A FLAT table is one `u32` each: **108 MiB, 42 % of the 256 MiB pool
//   it indexes**, to describe pages that are 99.99 % empty. Dead on
//   arrival, and it also kills the obvious allocator — a sweep over the
//   virtual space is a 28-million-thread dispatch to find ~2000 set
//   bits.
// - A HIERARCHICAL table is small, but it pays an indirection per
//   lookup, and the lookup is per pixel per light in the shading pass.
//   That is the hot path the froxel grid exists to keep short.
//
// So the table is sized to what is RESIDENT, not to what is
// addressable: open addressing over `2 x pool_pages` entries, which for
// Epic's 4096-page pool is 8192 slots — **64 KiB**, and one probe in the
// common case. UE5 hashes it too.
//
// # The insert has no race, and that is not luck
//
// Only the thread that flipped a page's mark bit from 0 to 1 ever
// inserts it — `mark_bit` already returns exactly that. So a key is
// claimed by one thread, and the compare-exchange below is there for
// DIFFERENT keys landing on the same slot, never for two threads
// fighting over one page. That is what makes the physical index safe to
// write with a plain store right after.

// 0 is EMPTY, so a cleared buffer is an empty table and no reset pass
// has to run. Keys are therefore stored as `page + 1`.
const PAGE_EMPTY: u32 = 0u;

/// No physical page: either the pool is full or the probe gave up.
const PAGE_MISS: u32 = 0xffffffffu;

/// How far a lookup walks before calling it a miss.
///
/// At a load factor of 0.5 the expected probe count is under 2; 32 is
/// the point where something is wrong with the hash rather than with
/// the load.
const PAGE_PROBES: u32 = 32u;

/// Murmur3's finalizer. Any bijection on 32 bits would do; what matters
/// is that page indices are DENSE and highly structured — consecutive
/// ids differ in the low bits and share every high one — so the low bits
/// alone would pile every page of a level onto one run of slots.
fn page_hash(key: u32) -> u32 {
    var h = key;
    h = h ^ (h >> 16u);
    h = h * 0x7feb352du;
    h = h ^ (h >> 15u);
    h = h * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return h;
}

/// Where a key's probe sequence starts. `entries` is a power of two.
fn page_probe(key: u32, entries: u32) -> u32 {
    return page_hash(key + 1u) & (entries - 1u);
}

/// The next slot in the sequence. Linear, because at load factor 0.5 the
/// clustering costs less than the cache misses a smarter sequence buys.
fn page_step(probe: u32, entries: u32) -> u32 {
    return (probe + 1u) & (entries - 1u);
}

/// The texel a physical page starts at, in the atlas.
///
/// The atlas is a plain grid of pages. `per_row` is a constant of the
/// pool, not of the light, so nothing about a page's ADDRESS survives
/// into its CONTENT — which is what lets a page be evicted and refilled
/// without anything that samples it noticing.
fn page_origin(slot: u32, per_row: u32, page: u32) -> vec2<u32> {
    return vec2<u32>(slot % per_row, slot / per_row) * page;
}
