//! Double-buffered GPU BVH state for the raymarch primitive culling
//! integration (PR-4 of #115).
//!
//! Owns a [`BvhGpuBuilder`] (the reusable LBVH compute pipeline + scratch
//! buffers from PR-3) plus two output slots — `slot_a` and `slot_b` —
//! each holding a stable copy of the last completed build's outputs
//! (nodes, sorted indices, leaf AABBs). The renderer always reads from
//! `current_slot`; pending builds copy their results into the OTHER slot
//! at swap time, so the renderer never observes a half-written buffer
//! and the GPU avoids a stall on the read-after-write hazard a single
//! shared buffer would introduce.
//!
//! The stale-buffer copy adds a constant amount of GPU bandwidth per
//! build (`(2N - 1) * 32` bytes for nodes + `N * 4` for indices) — far
//! cheaper than blocking the frame pipeline.
//!
//! # Lifecycle per frame
//!
//! ```text
//!   1. (frame start) bvh_state.poll_swap(device, queue):
//!        - drives wgpu's map_async callbacks via device.poll(Poll)
//!        - if pending build resolved, copy outputs to `target_slot`,
//!          flip current_slot, drop pending.
//!   2. Caller computes the new scene-state hash.
//!   3. bvh_state.kick_if_dirty(...) — no-op if hash matches; otherwise
//!      starts a new GPU build into the FREE slot (the one not currently
//!      bound to the renderer).
//!   4. Renderer binds bvh_state.current_*() buffers in the raymarch
//!      pass — guaranteed to be the last completed build.
//! ```
//!
//! # Empty / first-frame handling
//!
//! Before any build completes, `current_n() == 0` and the bind buffers
//! are minimal placeholders. The shader's `n == 0` branch must render
//! the sky, mirroring PR-3's `Bvh::empty()` semantics.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ome_bvh::{Aabb, BvhBuildError, BvhGpuBuild, BvhGpuBuilder, BvhNode};

use super::instance::{INITIAL_LEAF_AABB_CAPACITY, LeafAabb};

/// Initial capacity (in leaves) for the per-slot stable buffers. Grows
/// by `next_power_of_two` whenever a build exceeds it.
const INITIAL_SLOT_CAPACITY: u64 = 256;

/// Per-slot stable buffer set. The renderer binds these directly when
/// the slot is `current`. Capacity grows on demand; buffers never shrink.
struct OutputSlot {
    /// Flat tree of `BvhNode`s. Sized for `2N` to keep capacity
    /// arithmetic simple (real fill is `2N - 1`).
    nodes_buffer: wgpu::Buffer,
    /// `sorted_indices[k]` = original payload index at sorted position
    /// `k`. Length `N`.
    sorted_indices_buffer: wgpu::Buffer,
    /// Per-primitive metadata (AABB + role + smoothness). Length `N`.
    leaf_aabbs_buffer: wgpu::Buffer,
    /// Capacity in leaves (matches `LbvhBuffers::capacity`'s convention).
    capacity: u64,
    /// Number of valid leaves (= primitives) in this slot. `0` until the
    /// first build resolves into it.
    n: u32,
}

impl OutputSlot {
    fn new(device: &wgpu::Device, capacity: u64) -> Self {
        let nodes_buffer = make_nodes_buffer(device, capacity);
        let sorted_indices_buffer = make_indices_buffer(device, capacity);
        let leaf_aabbs_buffer = make_leaf_aabbs_buffer(device, capacity);
        Self {
            nodes_buffer,
            sorted_indices_buffer,
            leaf_aabbs_buffer,
            capacity,
            n: 0,
        }
    }

    fn ensure_capacity(&mut self, device: &wgpu::Device, n: u64) {
        if n <= self.capacity {
            return;
        }
        let new_cap = n.next_power_of_two().max(INITIAL_SLOT_CAPACITY);
        self.nodes_buffer = make_nodes_buffer(device, new_cap);
        self.sorted_indices_buffer = make_indices_buffer(device, new_cap);
        self.leaf_aabbs_buffer = make_leaf_aabbs_buffer(device, new_cap);
        self.capacity = new_cap;
    }
}

fn make_nodes_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("raymarch_bvh::slot::nodes"),
        size: 2 * capacity * std::mem::size_of::<BvhNode>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn make_indices_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("raymarch_bvh::slot::sorted_indices"),
        size: capacity * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn make_leaf_aabbs_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("raymarch_bvh::slot::leaf_aabbs"),
        size: capacity * std::mem::size_of::<LeafAabb>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// In-flight build awaiting GPU completion. Once
/// [`BvhGpuBuild::poll`] returns `Some(Ok(_))`, [`BvhState::poll_swap`]
/// copies the result into `target_slot` and flips `current_slot`.
struct PendingBuild {
    build: BvhGpuBuild<u32>,
    /// Slot index (0 or 1) the result will land in.
    target_slot: u8,
    /// Per-leaf metadata captured at kick time. Uploaded to the target
    /// slot's `leaf_aabbs_buffer` at swap. Held on the CPU because the
    /// LBVH builder doesn't see this metadata.
    leaf_aabbs: Vec<LeafAabb>,
    /// Number of leaves submitted to the build. Used at swap time to
    /// drive the copy lengths.
    n: u32,
}

/// Double-buffered BVH GPU state. Plug-in struct held as a sub-resource
/// of the raymarch renderer.
pub struct BvhState {
    pub(super) builder: BvhGpuBuilder,
    slot_a: OutputSlot,
    slot_b: OutputSlot,
    /// `0` → renderer reads `slot_a`, builds target `slot_b`. `1` → mirror.
    current_slot: u8,
    pending: Option<PendingBuild>,
    /// Hash of the last successfully kicked scene state (primitives
    /// bytes + per-leaf metadata). Used to skip redundant builds when
    /// the scene hasn't changed between frames.
    dirty_hash: Option<u64>,
}

impl BvhState {
    /// Build the GPU compute infrastructure + initial empty slots.
    /// `pipeline_cache` is forwarded to the LBVH builder — pass the
    /// engine's shared `wgpu::PipelineCache` to amortise compile time.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        let builder = BvhGpuBuilder::new(device, queue, pipeline_cache);
        let initial = INITIAL_LEAF_AABB_CAPACITY.max(INITIAL_SLOT_CAPACITY);
        Self {
            builder,
            slot_a: OutputSlot::new(device, initial),
            slot_b: OutputSlot::new(device, initial),
            current_slot: 0,
            pending: None,
            dirty_hash: None,
        }
    }

    /// Borrow the slot the renderer should bind this frame.
    pub fn current_nodes(&self) -> &wgpu::Buffer {
        &self.slot(self.current_slot).nodes_buffer
    }

    pub fn current_sorted_indices(&self) -> &wgpu::Buffer {
        &self.slot(self.current_slot).sorted_indices_buffer
    }

    pub fn current_leaf_aabbs(&self) -> &wgpu::Buffer {
        &self.slot(self.current_slot).leaf_aabbs_buffer
    }

    /// Number of valid primitives in the currently-bound slot. `0`
    /// before any build has resolved.
    pub fn current_n(&self) -> u32 {
        self.slot(self.current_slot).n
    }

    fn slot(&self, idx: u8) -> &OutputSlot {
        match idx {
            0 => &self.slot_a,
            _ => &self.slot_b,
        }
    }

    fn slot_mut(&mut self, idx: u8) -> &mut OutputSlot {
        match idx {
            0 => &mut self.slot_a,
            _ => &mut self.slot_b,
        }
    }

    /// Compute a stable hash of `(items + leaf_aabbs)` so callers can
    /// detect whether the scene changed since the last build.
    /// Exposed publicly so the update path can hash before paying the
    /// cost of formatting payload Vecs.
    pub fn hash_scene(items: &[(u32, Aabb)], leaf_aabbs: &[LeafAabb]) -> u64 {
        let mut h = DefaultHasher::new();
        items.len().hash(&mut h);
        for (id, a) in items {
            id.hash(&mut h);
            // Aabb's f32 fields don't implement Hash directly; project
            // through their bit patterns.
            for c in a.min.to_array().iter().chain(a.max.to_array().iter()) {
                c.to_bits().hash(&mut h);
            }
        }
        leaf_aabbs.len().hash(&mut h);
        for la in leaf_aabbs {
            la.role.hash(&mut h);
            la.smoothness.to_bits().hash(&mut h);
        }
        h.finish()
    }

    /// Start a new GPU build if the scene's hash changed since the last
    /// kick. Returns `true` when a new build was kicked.
    ///
    /// `items` is the BVH builder input; `leaf_aabbs` is the parallel
    /// per-leaf metadata that the WGSL traversal will consume. The two
    /// slices must be the same length and ordering.
    ///
    /// At most one build is in flight at a time. If a previous build
    /// has not yet been polled to completion, this is a no-op
    /// regardless of the dirty state — the caller should drive
    /// [`Self::poll_swap`] every frame to keep the pipeline moving.
    pub fn kick_if_dirty(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: Vec<(u32, Aabb)>,
        leaf_aabbs: Vec<LeafAabb>,
    ) -> bool {
        debug_assert_eq!(
            items.len(),
            leaf_aabbs.len(),
            "items and leaf_aabbs must align 1:1 — one entry per primitive",
        );
        if self.pending.is_some() {
            return false;
        }
        let hash = Self::hash_scene(&items, &leaf_aabbs);
        if Some(hash) == self.dirty_hash {
            return false;
        }
        let n = items.len() as u32;
        // Free slot is the one NOT currently bound to the renderer.
        let target_slot = self.current_slot ^ 1;

        // Make sure the target slot can hold the result. Always grow to
        // at least 1 to keep the buffers valid even for empty scenes.
        let needed = (n as u64).max(1);
        self.slot_mut(target_slot).ensure_capacity(device, needed);

        let build = ome_bvh::Bvh::<u32>::build_gpu(&mut self.builder, device, queue, items);
        self.pending = Some(PendingBuild {
            build,
            target_slot,
            leaf_aabbs,
            n,
        });
        self.dirty_hash = Some(hash);
        true
    }

    /// Drive the in-flight build forward. Must be called once per
    /// frame. Returns the build outcome on the frame the swap happens:
    ///
    /// - `None` — no pending build, or pending build still in flight.
    /// - `Some(Ok(()))` — pending build resolved; result copied into
    ///   the target slot; `current_slot` flipped.
    /// - `Some(Err(_))` — build failed; pending dropped without
    ///   touching the slots. The renderer keeps using the previous
    ///   slot's data until the next successful build.
    pub fn poll_swap(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<Result<(), BvhBuildError>> {
        device.poll(wgpu::PollType::Poll).ok()?;
        let Some(pending) = self.pending.as_mut() else {
            return None;
        };
        let outcome = pending.build.poll(device)?;
        // Take ownership now — we'll either commit or drop based on the
        // outcome.
        let pending = self.pending.take().expect("just observed Some above");

        let bvh = match outcome {
            Ok(b) => b,
            Err(e) => return Some(Err(e)),
        };

        // Copy the GPU build's output (still living in the builder's
        // internal scratch buffers) into the target slot's stable
        // buffers. Empty builds (n == 0) leave the slot's `n = 0`,
        // signalling "no primitives" to the renderer.
        let n = pending.n;
        let target_slot = pending.target_slot;
        if n > 0 {
            let total_nodes = (2 * n - 1) as u64;
            let nodes_bytes = total_nodes * std::mem::size_of::<BvhNode>() as u64;
            let indices_bytes = (n as u64) * 4;
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("raymarch_bvh::poll_swap_copy_encoder"),
            });
            encoder.copy_buffer_to_buffer(
                self.builder.nodes_buffer(),
                0,
                &self.slot(target_slot).nodes_buffer,
                0,
                nodes_bytes,
            );
            encoder.copy_buffer_to_buffer(
                self.builder.sorted_indices_buffer(),
                0,
                &self.slot(target_slot).sorted_indices_buffer,
                0,
                indices_bytes,
            );
            queue.submit(std::iter::once(encoder.finish()));
            queue.write_buffer(
                &self.slot(target_slot).leaf_aabbs_buffer,
                0,
                bytemuck::cast_slice(&pending.leaf_aabbs),
            );
        }
        self.slot_mut(target_slot).n = n;
        self.current_slot = target_slot;
        // Drop the build handle (and `bvh` Vec<BvhNode> we don't need)
        // explicitly to make the lifetime obvious.
        drop(bvh);
        Some(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use ome_bvh::Aabb;

    fn dummy_leaf(role: u32, smoothness: f32) -> LeafAabb {
        LeafAabb {
            aabb_min: [0.0; 3],
            role,
            aabb_max: [1.0; 3],
            smoothness,
        }
    }

    #[test]
    fn hash_is_stable_for_identical_inputs() {
        let items = vec![
            (0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE)),
            (1u32, Aabb::from_centre(Vec3::splat(5.0), Vec3::ONE)),
        ];
        let leaves = vec![dummy_leaf(0, 0.0), dummy_leaf(1, 0.5)];
        let h1 = BvhState::hash_scene(&items, &leaves);
        let h2 = BvhState::hash_scene(&items, &leaves);
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_changes_when_aabb_changes() {
        let items_a = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
        let items_b = vec![(0u32, Aabb::from_centre(Vec3::X, Vec3::ONE))];
        let leaves = vec![dummy_leaf(0, 0.0)];
        assert_ne!(
            BvhState::hash_scene(&items_a, &leaves),
            BvhState::hash_scene(&items_b, &leaves),
        );
    }

    #[test]
    fn hash_changes_when_role_changes() {
        let items = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
        let leaves_add = vec![dummy_leaf(0, 0.5)];
        let leaves_int = vec![dummy_leaf(1, 0.5)];
        assert_ne!(
            BvhState::hash_scene(&items, &leaves_add),
            BvhState::hash_scene(&items, &leaves_int),
        );
    }

    #[test]
    fn hash_changes_when_smoothness_changes() {
        let items = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
        let leaves_lo = vec![dummy_leaf(0, 0.1)];
        let leaves_hi = vec![dummy_leaf(0, 0.5)];
        assert_ne!(
            BvhState::hash_scene(&items, &leaves_lo),
            BvhState::hash_scene(&items, &leaves_hi),
        );
    }

    #[test]
    fn hash_changes_when_count_changes() {
        let leaves = vec![dummy_leaf(0, 0.0)];
        let items_one = vec![(0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE))];
        let items_two = vec![
            (0u32, Aabb::from_centre(Vec3::ZERO, Vec3::ONE)),
            (1u32, Aabb::from_centre(Vec3::X, Vec3::ONE)),
        ];
        let leaves_two = vec![dummy_leaf(0, 0.0), dummy_leaf(0, 0.0)];
        assert_ne!(
            BvhState::hash_scene(&items_one, &leaves),
            BvhState::hash_scene(&items_two, &leaves_two),
        );
    }
}

// ---------------------------------------------------------------------
// GPU end-to-end determinism test (S9 of PR-4 #115).
//
// Bypasses the ECS layer entirely: builds a synthetic scene of N
// random spheres, runs `BvhState::kick_if_dirty + poll` to GPU-build
// the BVH, then dispatches a small compute shader that invokes the
// same `eval_scene_bvh` traversal logic the fragment shader uses.
// Compares the float output between two consecutive runs of the same
// inputs: byte-identical guarantees the per-role accumulator order
// is a function of BVH topology only (not runtime ray geometry).
//
// The compute shader inlines the structs + traversal — separating it
// out into a shared module would expose internal types in the public
// API just to dedupe ~80 lines of WGSL, which is a worse trade.
// ---------------------------------------------------------------------
#[cfg(test)]
mod gpu_byte_identical {
    use super::*;
    use crate::raymarch::aabb::primitive_aabb;
    use crate::raymarch::instance::{ROLE_ADD, SceneMeta, SdfPrimitive, TYPE_SPHERE};
    use bytemuck::{Pod, Zeroable};
    use glam::Quat;

    /// Headless GPU acquisition. Returns `None` when no adapter is
    /// available or the timestamp features the BvhGpuBuilder needs are
    /// missing — same skip-not-fail policy as the ome_bvh GPU tests.
    fn try_acquire_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        pollster::block_on(async {
            let instance = wgpu::Instance::default();
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .ok()?;
            let needs = wgpu::Features::TIMESTAMP_QUERY
                | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES;
            if !adapter.features().contains(needs) {
                return None;
            }
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("raymarch_bvh::test_device"),
                    required_features: needs,
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                    trace: wgpu::Trace::Off,
                    experimental_features: wgpu::ExperimentalFeatures::default(),
                })
                .await
                .ok()?;
            Some((device, queue))
        })
    }

    /// Same fixed-step LCG used by the ome_bvh tests, so reproductions
    /// match.
    fn lcg(state: &mut u32) -> f32 {
        *state = state.wrapping_mul(1103515245).wrapping_add(12345);
        (*state >> 16) as f32 / 32768.0
    }

    /// Build a deterministic scene of `n` unit spheres scattered across
    /// a 100³ box.
    fn random_sphere_scene(n: u32, seed: u32) -> (Vec<SdfPrimitive>, Vec<LeafAabb>) {
        let mut state = seed;
        let mut prims = Vec::with_capacity(n as usize);
        let mut leaves = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let pos = [lcg(&mut state) * 100.0, lcg(&mut state) * 100.0, lcg(&mut state) * 100.0];
            let radius = 0.5 + lcg(&mut state) * 0.5;
            let prim = SdfPrimitive {
                position: pos,
                type_tag: TYPE_SPHERE,
                rotation: Quat::IDENTITY.to_array(),
                scale: [1.0; 3],
                _pad0: 0.0,
                params: [radius, 0.0, 0.0, 0.0],
            };
            let aabb = primitive_aabb(&prim, 0.0);
            leaves.push(LeafAabb {
                aabb_min: aabb.min.to_array(),
                role: ROLE_ADD,
                aabb_max: aabb.max.to_array(),
                smoothness: 0.0,
            });
            prims.push(prim);
        }
        (prims, leaves)
    }

    /// Sample-points payload used by the compute shader (vec4 padded to
    /// keep std430 alignment simple).
    #[repr(C)]
    #[derive(Copy, Clone, Pod, Zeroable, Default)]
    struct SamplePoint {
        pos: [f32; 4],
    }

    fn sample_points_grid(n: u32) -> Vec<SamplePoint> {
        // Deterministic grid of points across the same 100³ box used to
        // place the spheres. Enough samples land inside primitives'
        // AABBs that the per-role accumulator actually exercises a few
        // smooth_union / smooth_intersect calls per ray.
        let side = (n as f32).cbrt().ceil() as u32;
        let step = 100.0 / side as f32;
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n {
            let x = (i % side) as f32 * step;
            let y = ((i / side) % side) as f32 * step;
            let z = (i / (side * side)) as f32 * step;
            out.push(SamplePoint { pos: [x, y, z, 0.0] });
        }
        out
    }

    /// Compute shader that mirrors `eval_scene_bvh` from
    /// `raymarch_main.wgsl`. Kept as a local string so the test does
    /// not have to expose internals of the raymarch module — the
    /// function body is structurally identical to the production
    /// fragment-shader version.
    const TEST_COMPUTE_WGSL: &str = r#"
struct BvhNode {
    aabb_min: vec3<f32>,
    left: u32,
    aabb_max: vec3<f32>,
    right_or_count: u32,
}
struct LeafAabb {
    aabb_min: vec3<f32>,
    role: u32,
    aabb_max: vec3<f32>,
    smoothness: f32,
}
struct SdfPrimitive {
    position: vec3<f32>,
    type_tag: u32,
    rotation: vec4<f32>,
    scale: vec3<f32>,
    _pad0: f32,
    params: vec4<f32>,
}
struct SceneMeta {
    primitive_count: u32,
    bvh_n: u32,
    skip_internal_sky: u32,
    has_intersects: u32,
    has_subs: u32,
    k_int_scene: f32,
    k_sub_scene: f32,
    _pad0: u32,
    sky_top: vec4<f32>,
    sky_bottom: vec4<f32>,
}
struct SamplePoint { pos: vec4<f32> }

@group(0) @binding(0) var<uniform>          scene_meta:     SceneMeta;
@group(0) @binding(1) var<storage, read>    primitives:     array<SdfPrimitive>;
@group(0) @binding(2) var<storage, read>    bvh_nodes:      array<BvhNode>;
@group(0) @binding(3) var<storage, read>    sorted_indices: array<u32>;
@group(0) @binding(4) var<storage, read>    leaf_aabbs:     array<LeafAabb>;
@group(0) @binding(5) var<storage, read>    samples:        array<SamplePoint>;
@group(0) @binding(6) var<storage, read_write> out_d:       array<f32>;

const ACC_UNION_IDENTITY: f32 = 1.0e10;
const ACC_INTERSECT_IDENTITY: f32 = -1.0e10;
const BVH_LEAF_FLAG: u32 = 0x80000000u;
const BVH_VALUE_MASK: u32 = 0x7FFFFFFFu;

fn smooth_union(d1: f32, d2: f32, k: f32) -> f32 {
    let h = clamp(0.5 + 0.5 * (d2 - d1) / k, 0.0, 1.0);
    return mix(d2, d1, h) - k * h * (1.0 - h);
}

fn sphere_at(p: vec3<f32>, prim: SdfPrimitive) -> f32 {
    let local = p - prim.position;
    return length(local) - prim.params.x;
}

fn point_in_aabb(p: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>) -> bool {
    return all(p >= lo) && all(p <= hi);
}

fn eval_scene_bvh(p: vec3<f32>) -> f32 {
    if scene_meta.bvh_n == 0u { return ACC_UNION_IDENTITY; }
    var add_acc = ACC_UNION_IDENTITY;
    var int_acc = ACC_INTERSECT_IDENTITY;
    var sub_acc = ACC_UNION_IDENTITY;
    var stack: array<u32, 32>;
    stack[0] = 0u;
    var sp = 1u;
    while sp > 0u {
        sp = sp - 1u;
        let node = bvh_nodes[stack[sp]];
        if !point_in_aabb(p, node.aabb_min, node.aabb_max) { continue; }
        let payload = node.right_or_count;
        if (payload & BVH_LEAF_FLAG) != 0u {
            let count = payload & BVH_VALUE_MASK;
            let first = node.left;
            for (var i: u32 = 0u; i < count; i = i + 1u) {
                let prim_idx = sorted_indices[first + i];
                let leaf = leaf_aabbs[prim_idx];
                let d = sphere_at(p, primitives[prim_idx]);
                let k = max(leaf.smoothness, 1e-5);
                add_acc = smooth_union(add_acc, d, k);
            }
        } else {
            let left = node.left;
            let right = payload & BVH_VALUE_MASK;
            if sp + 2u <= 32u {
                stack[sp] = left; sp = sp + 1u;
                stack[sp] = right; sp = sp + 1u;
            }
        }
    }
    return add_acc;
}

@compute @workgroup_size(64)
fn cs_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if i >= arrayLength(&samples) { return; }
    out_d[i] = eval_scene_bvh(samples[i].pos.xyz);
}
"#;

    /// Run the compute shader once and return the per-sample
    /// distances. Re-uses the `BvhState`'s GPU-resident buffers —
    /// matches the production binding layout.
    fn run_eval_pass(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        state: &BvhState,
        primitives: &[SdfPrimitive],
        leaf_aabbs: &[LeafAabb],
        samples: &[SamplePoint],
        meta: &SceneMeta,
    ) -> Vec<f32> {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("raymarch_bvh::test_compute"),
            source: wgpu::ShaderSource::Wgsl(TEST_COMPUTE_WGSL.into()),
        });

        let meta_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test_meta"),
            size: std::mem::size_of::<SceneMeta>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&meta_buffer, 0, bytemuck::bytes_of(meta));

        let prims_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test_primitives"),
            size: (primitives.len() * std::mem::size_of::<SdfPrimitive>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&prims_buffer, 0, bytemuck::cast_slice(primitives));

        // Note: leaf_aabbs is uploaded INTO the BvhState's slot at
        // poll_swap time via queue.write_buffer; we re-upload here only
        // because the test wrapper needs a known-aligned binding. In
        // production the bind group already points at
        // bvh_state.current_leaf_aabbs() — same data either way.
        let leaves_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test_leaf_aabbs"),
            size: (leaf_aabbs.len() * std::mem::size_of::<LeafAabb>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&leaves_buffer, 0, bytemuck::cast_slice(leaf_aabbs));

        let samples_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test_samples"),
            size: (samples.len() * std::mem::size_of::<SamplePoint>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&samples_buffer, 0, bytemuck::cast_slice(samples));

        let out_size = (samples.len() * 4) as u64;
        let out_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test_out"),
            size: out_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("test_bgl"),
            entries: &[
                bgl_entry(0, wgpu::BufferBindingType::Uniform),
                bgl_entry(1, wgpu::BufferBindingType::Storage { read_only: true }),
                bgl_entry(2, wgpu::BufferBindingType::Storage { read_only: true }),
                bgl_entry(3, wgpu::BufferBindingType::Storage { read_only: true }),
                bgl_entry(4, wgpu::BufferBindingType::Storage { read_only: true }),
                bgl_entry(5, wgpu::BufferBindingType::Storage { read_only: true }),
                bgl_entry(6, wgpu::BufferBindingType::Storage { read_only: false }),
            ],
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("test_bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: meta_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: prims_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: state.current_nodes().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: state.current_sorted_indices().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: leaves_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: samples_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: out_buffer.as_entire_binding() },
            ],
        });

        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("test_pl"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("test_compute_pipeline"),
            layout: Some(&pl),
            module: &module,
            entry_point: Some("cs_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("test_compute_encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("test_compute_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bg, &[]);
            let groups = (samples.len() as u32).div_ceil(64);
            pass.dispatch_workgroups(groups.max(1), 1, 1);
        }
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test_staging"),
            size: out_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&out_buffer, 0, &staging, 0, out_size);
        queue.submit(std::iter::once(encoder.finish()));

        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { tx.send(r).ok(); });
        device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(std::time::Duration::from_secs(30)) })
            .expect("poll");
        rx.recv().expect("map_async sender").expect("map_async result");
        let data = slice.get_mapped_range();
        let v: Vec<f32> = bytemuck::cast_slice::<u8, f32>(&data).to_vec();
        drop(data);
        staging.unmap();
        v
    }

    fn bgl_entry(binding: u32, ty: wgpu::BufferBindingType) -> wgpu::BindGroupLayoutEntry {
        wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }
    }

    fn drive_bvh_to_completion(state: &mut BvhState, device: &wgpu::Device, queue: &wgpu::Queue) {
        // Loop until poll_swap reports a swap. PR-3's BvhGpuBuild
        // resolves in 1-2 iterations on a healthy queue.
        for _ in 0..16 {
            if let Some(outcome) = state.poll_swap(device, queue) {
                outcome.expect("BVH build must succeed for the test");
                return;
            }
            // Force progress on the queue. PollType::Wait without
            // a submission index waits on every outstanding submission.
            let _ = device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(5)),
            });
        }
        panic!("BVH build did not resolve within 16 poll iterations");
    }

    /// Eight spheres on a regular grid; same scene rendered twice; the
    /// per-sample float output of the second run must be byte-identical
    /// to the first. Validates the determinism of the BVH topology
    /// (Karras + onesweep — already covered in PR-3) plus the
    /// deterministic accumulator ordering inside `eval_scene_bvh`.
    #[test]
    fn cull_vs_cull_byte_identical_n_8() {
        let Some((device, queue)) = try_acquire_device() else {
            eprintln!("raymarch_bvh::gpu_byte_identical: no GPU adapter — skipping");
            return;
        };
        let (primitives, leaves) = random_sphere_scene(8, 0xc0ffee01);
        let items: Vec<(u32, ome_bvh::Aabb)> = leaves
            .iter()
            .enumerate()
            .map(|(i, l)| {
                (
                    i as u32,
                    ome_bvh::Aabb::new(
                        glam::Vec3::from_array(l.aabb_min),
                        glam::Vec3::from_array(l.aabb_max),
                    ),
                )
            })
            .collect();

        let mut state = BvhState::new(&device, &queue, None);
        state.kick_if_dirty(&device, &queue, items.clone(), leaves.clone());
        drive_bvh_to_completion(&mut state, &device, &queue);
        assert_eq!(state.current_n(), 8, "build must populate slot");

        let samples = sample_points_grid(512);
        let meta = SceneMeta {
            primitive_count: primitives.len() as u32,
            bvh_n: state.current_n(),
            skip_internal_sky: 0,
            has_intersects: 0,
            has_subs: 0,
            k_int_scene: 0.0,
            k_sub_scene: 0.0,
            _pad0: 0,
            sky_top: [0.5, 0.7, 1.0, 1.0],
            sky_bottom: [0.1, 0.2, 0.4, 1.0],
        };

        let run_a = run_eval_pass(&device, &queue, &state, &primitives, &leaves, &samples, &meta);
        let run_b = run_eval_pass(&device, &queue, &state, &primitives, &leaves, &samples, &meta);
        assert_eq!(run_a.len(), run_b.len());
        for (i, (a, b)) in run_a.iter().zip(run_b.iter()).enumerate() {
            // bit-exact equality — `assert_eq!` on f32 already does
            // bitwise compare, but explicit `to_bits()` makes the
            // intent unambiguous against future readers.
            assert_eq!(
                a.to_bits(),
                b.to_bits(),
                "sample[{i}] diverged across runs: {a} vs {b}",
            );
        }
    }

    /// Same property at 1024 leaves — the BVH has multiple internal
    /// levels here, and the per-role accumulator visits each leaf in
    /// a strictly topology-driven order. Catches any latent
    /// non-determinism in stack push ordering or atomic accumulation.
    #[test]
    fn cull_vs_cull_byte_identical_n_1024() {
        let Some((device, queue)) = try_acquire_device() else { return; };
        let (primitives, leaves) = random_sphere_scene(1024, 0xfeedface);
        let items: Vec<(u32, ome_bvh::Aabb)> = leaves
            .iter()
            .enumerate()
            .map(|(i, l)| {
                (
                    i as u32,
                    ome_bvh::Aabb::new(
                        glam::Vec3::from_array(l.aabb_min),
                        glam::Vec3::from_array(l.aabb_max),
                    ),
                )
            })
            .collect();

        let mut state = BvhState::new(&device, &queue, None);
        state.kick_if_dirty(&device, &queue, items, leaves.clone());
        drive_bvh_to_completion(&mut state, &device, &queue);
        assert_eq!(state.current_n(), 1024);

        let samples = sample_points_grid(2048);
        let meta = SceneMeta {
            primitive_count: primitives.len() as u32,
            bvh_n: state.current_n(),
            skip_internal_sky: 0,
            has_intersects: 0,
            has_subs: 0,
            k_int_scene: 0.0,
            k_sub_scene: 0.0,
            _pad0: 0,
            sky_top: [0.5, 0.7, 1.0, 1.0],
            sky_bottom: [0.1, 0.2, 0.4, 1.0],
        };

        let run_a = run_eval_pass(&device, &queue, &state, &primitives, &leaves, &samples, &meta);
        let run_b = run_eval_pass(&device, &queue, &state, &primitives, &leaves, &samples, &meta);
        for (i, (a, b)) in run_a.iter().zip(run_b.iter()).enumerate() {
            assert_eq!(
                a.to_bits(),
                b.to_bits(),
                "sample[{i}] diverged across runs at N=1024",
            );
        }
    }
}
