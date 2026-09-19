// masked_bins.wgsl — the visible meshlets of masked materials, grouped per material (#452) so each
// material's raster draws only its own: count, offsets, scatter, the order Nanite bins rasters in.
// Opaque meshlets are left to the cull's own draw, which skips masked instances.

struct DrawArgs {
    vertex_count: u32,
    instance_count: u32,
    first_vertex: u32,
    first_instance: u32,
}

struct MeshInstance {
    transform: mat4x4<f32>,
    mesh_id: u32,
    material_id: u32,
    lod_bias: f32,
    lod_force_level: i32,
    group_base: u32,
    flags: u32,
    _pad1: u32,
    _pad2: u32,
}

const BINS: u32 = 32u;
const INSTANCE_MASKED: u32 = 8u;
const GROUP: u32 = 64u;
const MAX_GROUPS: u32 = 65535u;

@group(0) @binding(0) var<storage, read> visible_meshlets: array<u32>;
// The cull's draw: vertices per meshlet, then how many survived.
@group(0) @binding(1) var<storage, read> cull_args: array<u32>;
@group(0) @binding(2) var<storage, read> instances: array<MeshInstance>;
// Per material slot: its bin + 1, or 0 when it is not masked this frame.
@group(0) @binding(3) var<storage, read> bin_of: array<u32>;
// Counts, then cursors once the offsets are taken.
@group(0) @binding(4) var<storage, read_write> counts: array<atomic<u32>, BINS>;
@group(0) @binding(5) var<storage, read_write> bases: array<u32, BINS>;
@group(0) @binding(6) var<storage, read_write> args: array<DrawArgs, BINS>;
// Slots into `visible_meshlets`, bin after bin.
@group(0) @binding(7) var<storage, read_write> slots: array<u32>;
// A group of its own: bound while the dispatches that read it as indirect args run, it would be a
// storage write in their scope.
@group(1) @binding(0) var<storage, read_write> dispatch: array<u32, 3>;

fn visible() -> u32 {
    return min(cull_args[1], arrayLength(&visible_meshlets));
}

// The bin a visible slot draws in, or BINS when it is not masked.
fn bin_at(slot: u32) -> u32 {
    let inst = instances[visible_meshlets[slot] >> 16u];
    if ((inst.flags & INSTANCE_MASKED) == 0u || inst.material_id >= arrayLength(&bin_of)) {
        return BINS;
    }
    let bin = bin_of[inst.material_id];
    return select(BINS, bin - 1u, bin != 0u && bin <= BINS);
}

@compute @workgroup_size(BINS)
fn cs_prepare(@builtin(local_invocation_index) lane: u32) {
    atomicStore(&counts[lane], 0u);
    if (lane == 0u) {
        dispatch[0] = clamp((visible() + GROUP - 1u) / GROUP, 1u, MAX_GROUPS);
        dispatch[1] = 1u;
        dispatch[2] = 1u;
    }
}

@compute @workgroup_size(GROUP)
fn cs_count(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    // Strided: past 65 535 groups one dispatch cannot give every slot a thread.
    let stride = groups.x * GROUP;
    for (var slot = id.x; slot < visible(); slot += stride) {
        let bin = bin_at(slot);
        if (bin < BINS) {
            atomicAdd(&counts[bin], 1u);
        }
    }
}

@compute @workgroup_size(1)
fn cs_offsets() {
    var base = 0u;
    for (var bin = 0u; bin < BINS; bin++) {
        let count = atomicLoad(&counts[bin]);
        bases[bin] = base;
        args[bin] = DrawArgs(cull_args[0], count, 0u, 0u);
        atomicStore(&counts[bin], 0u);
        base += count;
    }
}

@compute @workgroup_size(GROUP)
fn cs_scatter(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let stride = groups.x * GROUP;
    for (var slot = id.x; slot < visible(); slot += stride) {
        let bin = bin_at(slot);
        if (bin < BINS) {
            slots[bases[bin] + atomicAdd(&counts[bin], 1u)] = slot;
        }
    }
}
