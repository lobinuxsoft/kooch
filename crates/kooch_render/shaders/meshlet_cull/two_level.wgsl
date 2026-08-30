
// ---------------------------------------------------------------
// Two-level cull (#1002). Reject INSTANCES first, then expand only
// the survivors into meshlets.
//
// The one-level pool cull dispatches a rectangle:
// `instance_count × pool.max_meshlets_per_mesh`. That stride is the
// heaviest mesh registered ANYWHERE, so a field of one-meshlet cubes
// pays the dragon's 4755 threads apiece to die in a bounds check.
// Measured on `dense.scene`: 9 633 630 threads for ~116 000 real
// meshlets, 98.8 % of them padding — and contagious, because
// importing one detailed prop slows down every other instance.
//
// Here the meshlet domain is entered per instance, at that instance's
// OWN `meshlet_count`, through an indirect dispatch the first pass
// sizes. `page_expand.wgsl` already works this way for the pages; the
// main cull is the pass that never got it.
//
// The two meshlet-domain entries are the SAME bodies the rectangle
// entries run — `lod_group_max_err` and `cull_pool_atomic`, both split
// out of `atomic.wgsl`. Only how a thread finds its meshlet changed.
// A second copy of the LOD descent would be a second answer, and the
// two passes disagreeing is a group descending half-way, which reads
// as a hole in a surface.
//
// 🔴 The residual padding is a CONSTANT, not the scene's maximum. A
// surviving instance rounds up to a whole workgroup, so it wastes at
// most `CULL_GROUP - 1` lanes no matter what else is registered.
// Packing several small instances into one workgroup is the next
// step and needs a work queue; bounding the waste by 63 is what
// stops this from being a scaling bug.
// ---------------------------------------------------------------

const CULL_GROUP: u32 = 64u;

// Mirrors `MAX_WORKGROUPS_PER_DIM` in `kooch_core::gpu::limits`. The
// indirect args tile into y the same way `tiled_workgroups` does on
// the CPU, because a scene can ask for more workgroups than one
// dimension holds.
const MAX_GROUPS_PER_DIM: u32 = 65535u;

// `chunks` is ONE buffer because the layout is already at seven of
// the eight storage buffers a compute stage may bind. Words:
//
//   [0] chunk count, the atomic every survivor bumps
//   [1] chunks dropped over capacity — a diagnostic, never a clamp
//   [2] instances that survived, for the stats panel
//   [3..6) the indirect dispatch args `cs_cull_expand` runs under
//   [CHUNK_LIST..] one word per chunk: `(instance << 8) | chunk_index`
//
// WGSL forbids a mixed array, so the plain words are `atomicStore`d.
// The indirect read sees the same bytes either way.
const CHUNK_COUNT: u32 = 0u;
const CHUNK_DROPPED: u32 = 1u;
const CHUNK_INSTANCES: u32 = 2u;
const CHUNK_ARGS: u32 = 3u;
const CHUNK_LIST: u32 = 6u;

// A chunk word packs the instance in the high 24 bits and the chunk
// index within that instance in the low 8. 256 chunks is 16 384
// meshlets in one mesh, over three times the heaviest asset in the
// tree, and the instance field is wider than the 16 bits
// `visible_meshlets` already caps at.
const CHUNK_INDEX_BITS: u32 = 8u;
const CHUNK_INDEX_MASK: u32 = 255u;
const MAX_CHUNKS_PER_INSTANCE: u32 = 256u;

// (center, radius) per MESH, parallel to the descriptors — the same
// buffer `lamp_cull` culls instances with. See
// `GpuGlobalMeshPool::mesh_bounds`.
// 🔴 Bindings 1 and 2, never 0: `atomic.wgsl` declares `group_max_err`
// at group(3) binding(0) and one module cannot hold two globals at the
// same address. The chunked entries read both.
@group(3) @binding(1) var<storage, read> mesh_bounds: array<vec4<f32>>;
@group(3) @binding(2) var<storage, read_write> chunks: array<atomic<u32>>;

/// How many pixels across the instance's bounding sphere covers.
///
/// The same projection `lod_pixel_error` uses, and deliberately so:
/// `lod_error_to_pixel_factor` is already `0.5 · viewport_h ·
/// proj_scale_y`, so a radius over a distance comes out in the units
/// the LOD threshold is authored in.
fn instance_screen_pixels(centre: vec3<f32>, radius: f32) -> f32 {
    // An orthographic view — a shadow cascade — does not shrink
    // anything with distance, so the screen size is the world size and
    // there is no divide. Same branch, same reason, as the LOD test.
    if params.lod_orthographic == 1u {
        return radius * params.lod_error_to_pixel_factor;
    }
    let dist = max(length(centre - params.camera_position), 0.0001);
    return radius * params.lod_error_to_pixel_factor / dist;
}

// One thread per INSTANCE. Frustum, then screen coverage. A survivor
// reserves as many chunks as its own mesh needs and writes them.
@compute @workgroup_size(CULL_GROUP, 1, 1)
fn cs_cull_instances(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let instance_id = linear_thread(gid, groups);
    if instance_id >= scene_params.instance_count {
        return;
    }

    let inst = instances[instance_id];
    let mesh_desc = pool_mesh_descriptors[inst.mesh_id];
    if mesh_desc.meshlet_count == 0u {
        return;
    }

    let bounds = mesh_bounds[inst.mesh_id];
    let centre = (inst.transform * vec4<f32>(bounds.xyz, 1.0)).xyz;
    let radius = bounds.w * instance_world_scale(inst.transform);

    if sphere_outside_frustum(centre, radius) {
        record_reject(rectangle_slot(instance_id, 0u), REJECT_REASON_INSTANCE);
        return;
    }

    // 🔴 The reach test, and the reason it lives HERE rather than in
    // the meshlet domain: rejecting an instance costs one thread,
    // rejecting its meshlets costs one per meshlet. A threshold of 0
    // is off, which is what ships until something has measured what a
    // non-zero one hides.
    if params.min_screen_pixels > 0.0
        && instance_screen_pixels(centre, radius) < params.min_screen_pixels
    {
        record_reject(rectangle_slot(instance_id, 0u), REJECT_REASON_REACH);
        return;
    }

    atomicAdd(&chunks[CHUNK_INSTANCES], 1u);

    let wanted = min(
        (mesh_desc.meshlet_count + CULL_GROUP - 1u) / CULL_GROUP,
        MAX_CHUNKS_PER_INSTANCE,
    );
    let base = atomicAdd(&chunks[CHUNK_COUNT], wanted);
    let capacity = scene_params.chunk_capacity;
    for (var i = 0u; i < wanted; i = i + 1u) {
        let slot = base + i;
        if slot >= capacity {
            atomicAdd(&chunks[CHUNK_DROPPED], 1u);
            continue;
        }
        atomicStore(
            &chunks[CHUNK_LIST + slot],
            (instance_id << CHUNK_INDEX_BITS) | i,
        );
    }
}

// Sizes the expansion from a number that only exists on the GPU.
@compute @workgroup_size(1, 1, 1)
fn cs_cull_expand_args() {
    let counted = min(atomicLoad(&chunks[CHUNK_COUNT]), scene_params.chunk_capacity);
    var x = counted;
    var y = 1u;
    if counted > MAX_GROUPS_PER_DIM {
        x = MAX_GROUPS_PER_DIM;
        y = (counted + MAX_GROUPS_PER_DIM - 1u) / MAX_GROUPS_PER_DIM;
    }
    atomicStore(&chunks[CHUNK_ARGS], x);
    atomicStore(&chunks[CHUNK_ARGS + 1u], y);
    atomicStore(&chunks[CHUNK_ARGS + 2u], 1u);
}

/// Which (instance, meshlet_offset) a lane of a chunk workgroup owns.
///
/// `.z` is 0 when the lane has no meshlet — past the chunk count, or
/// past this instance's own meshlet count. Every chunked entry starts
/// with this and returns on a zero, which is what keeps the two passes
/// walking exactly the same set.
fn chunk_target(chunk_id: u32, lane: u32) -> vec3<u32> {
    let counted = min(atomicLoad(&chunks[CHUNK_COUNT]), scene_params.chunk_capacity);
    if chunk_id >= counted {
        return vec3<u32>(0u, 0u, 0u);
    }
    let packed_chunk = atomicLoad(&chunks[CHUNK_LIST + chunk_id]);
    let instance_id = packed_chunk >> CHUNK_INDEX_BITS;
    let meshlet_offset = (packed_chunk & CHUNK_INDEX_MASK) * CULL_GROUP + lane;
    let mesh_desc = pool_mesh_descriptors[instances[instance_id].mesh_id];
    if meshlet_offset >= mesh_desc.meshlet_count {
        return vec3<u32>(0u, 0u, 0u);
    }
    return vec3<u32>(instance_id, meshlet_offset, 1u);
}

/// The index this (instance, meshlet) pair would have had under the
/// one-level rectangle.
///
/// 🔴 Only `reject_reasons` is addressed this way, and deliberately:
/// the overlay decodes a slot back into an instance and a meshlet with
/// `thread_id / meshlets_per_mesh`, so keeping the debug buffer in the
/// old domain leaves it working untouched. It costs one multiply on a
/// path that already loaded the descriptor, and it is the only place
/// `meshlets_per_mesh` still appears.
fn rectangle_slot(instance_id: u32, meshlet_offset: u32) -> u32 {
    return instance_id * scene_params.meshlets_per_mesh + meshlet_offset;
}

/// The linear chunk a workgroup owns, tiled into y the way
/// `cs_cull_expand_args` writes the dispatch.
fn linear_chunk(wid: vec3<u32>, groups: vec3<u32>) -> u32 {
    return wid.y * groups.x + wid.x;
}

// Pass 1 of #465, reached from a chunk. One workgroup per chunk, one
// lane per meshlet of that chunk.
@compute @workgroup_size(CULL_GROUP, 1, 1)
fn cs_lod_group_max_err_chunked(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
) {
    let owned = chunk_target(linear_chunk(wid, groups), lid.x);
    if owned.z == 0u {
        return;
    }
    lod_group_max_err(owned.x, owned.y);
}

// Pass 2 of #465, reached from a chunk.
//
// Every lane of a workgroup shares an instance, so the transform, the
// mesh descriptor and the LOD scale are uniform across it — which is
// the second reason to slice by chunk and not by a flat thread index.
@compute @workgroup_size(CULL_GROUP, 1, 1)
fn cs_cull_scene_pool_atomic_chunked(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
) {
    let owned = chunk_target(linear_chunk(wid, groups), lid.x);
    if owned.z == 0u {
        return;
    }
    cull_pool_atomic(rectangle_slot(owned.x, owned.y), owned.x, owned.y);
}
