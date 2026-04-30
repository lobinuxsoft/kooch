//! Per-slot raymarch-only side buffers parallel to the
//! [`ome_bvh::SharedBvhState`] slot rotation.
//!
//! Two double-buffers ride alongside the shared `(nodes, sorted_indices,
//! leaf_aabbs)` triplet:
//!
//! - [`PayloadSlot`] — `RaymarchPayload[]` (smoothness). Bound at
//!   raymarch fragment binding 5.
//! - [`PrimitiveSlot`] — `SdfPrimitive[]` (position / rotation / scale /
//!   type / params). Bound at raymarch fragment binding 1.
//!
//! Both upload through the orchestrator's [`BuildToken::attach_payload`]
//! mechanism: the captured `Vec` rides the build that just kicked, fires
//! once on swap success, and is dropped without running on swap failure.
//! Net result — every slot's `(BVH, leaf_aabbs, RaymarchPayload,
//! SdfPrimitive)` tuple is consistent with itself, regardless of how
//! many builds are in flight.
//!
//! This is the lockstep contract that fixes #356: the fragment shader
//! never reads `primitives[i]` at the new position while the BVH cull
//! still sees `leaf_aabbs[i]` at the old position. Either every consumer
//! of the slot reflects the new state, or none of them do.

use ome_bvh::BuildToken;

use crate::raymarch::instance::{RaymarchPayload, SdfPrimitive};

/// Initial capacity (in primitives) for the raymarch-side double-
/// buffers. Tracks `INITIAL_SLOT_CAPACITY` in `ome_bvh::shared` so
/// growth events line up across every parallel buffer.
pub(super) const INITIAL_SIDE_CAPACITY: u64 = 256;

/// `RaymarchPayload[]` per slot. Mirrors the shared BVH's slot
/// rotation; bound by the raymarch fragment shader at binding 5.
pub(super) struct PayloadSlot {
    pub(super) buffer: wgpu::Buffer,
    capacity: u64,
}

impl PayloadSlot {
    pub(super) fn new(device: &wgpu::Device, capacity: u64) -> Self {
        Self {
            buffer: make_buffer::<RaymarchPayload>(device, capacity, "raymarch_bvh::payload_slot"),
            capacity,
        }
    }

    fn ensure_capacity(&mut self, device: &wgpu::Device, n: u64) {
        if n <= self.capacity {
            return;
        }
        let new_cap = n.next_power_of_two().max(INITIAL_SIDE_CAPACITY);
        self.buffer = make_buffer::<RaymarchPayload>(device, new_cap, "raymarch_bvh::payload_slot");
        self.capacity = new_cap;
    }
}

/// `SdfPrimitive[]` per slot. Same lifecycle as [`PayloadSlot`]; bound
/// by the raymarch fragment shader at binding 1. Slot-rotated so the
/// BVH cull and the SDF evaluation always read the same scene state.
pub(super) struct PrimitiveSlot {
    pub(super) buffer: wgpu::Buffer,
    capacity: u64,
}

impl PrimitiveSlot {
    pub(super) fn new(device: &wgpu::Device, capacity: u64) -> Self {
        Self {
            buffer: make_buffer::<SdfPrimitive>(device, capacity, "raymarch_bvh::primitive_slot"),
            capacity,
        }
    }

    fn ensure_capacity(&mut self, device: &wgpu::Device, n: u64) {
        if n <= self.capacity {
            return;
        }
        let new_cap = n.next_power_of_two().max(INITIAL_SIDE_CAPACITY);
        self.buffer =
            make_buffer::<SdfPrimitive>(device, new_cap, "raymarch_bvh::primitive_slot");
        self.capacity = new_cap;
    }
}

/// Grow the target slot to fit the kicked build's `n`, then register
/// the upload closure on the orchestrator's [`BuildToken`]. The
/// closure captures a refcounted clone of the (possibly freshly-grown)
/// `wgpu::Buffer` plus the owned `Vec` payload, so any later regrow
/// on the consumer side cannot redirect the upload to the wrong buffer
/// — the swap publishes whatever the kick committed to.
pub(super) fn attach_payload_upload(
    payload_slots: &mut [PayloadSlot; 2],
    device: &wgpu::Device,
    token: &mut BuildToken<'_>,
    raymarch_payloads: Vec<RaymarchPayload>,
) {
    let target_slot = token.target_slot();
    let n = token.n();
    let needed = (n as u64).max(1);
    payload_slots[target_slot as usize].ensure_capacity(device, needed);
    let buf = payload_slots[target_slot as usize].buffer.clone();
    token.attach_payload(move |queue, _slot| {
        queue.write_buffer(&buf, 0, bytemuck::cast_slice(&raymarch_payloads));
    });
}

/// `SdfPrimitive` analogue of [`attach_payload_upload`]. Same
/// invariants — the captured buffer and `Vec` ride the build that just
/// kicked, the upload fires on swap success, the captures drop on
/// failure.
pub(super) fn attach_primitive_upload(
    primitive_slots: &mut [PrimitiveSlot; 2],
    device: &wgpu::Device,
    token: &mut BuildToken<'_>,
    primitives: Vec<SdfPrimitive>,
) {
    let target_slot = token.target_slot();
    let n = token.n();
    let needed = (n as u64).max(1);
    primitive_slots[target_slot as usize].ensure_capacity(device, needed);
    let buf = primitive_slots[target_slot as usize].buffer.clone();
    token.attach_payload(move |queue, _slot| {
        if !primitives.is_empty() {
            queue.write_buffer(&buf, 0, bytemuck::cast_slice(&primitives));
        }
    });
}

fn make_buffer<T>(device: &wgpu::Device, capacity: u64, label: &str) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: capacity * std::mem::size_of::<T>() as u64,
        // COPY_SRC lets the regression suite read back side-buffers
        // (#356 lockstep tests). Production cost is zero — wgpu does
        // not allocate extra residency for usage flags it never
        // exercises.
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}
