// Pass 1 of 4: which slices of the grid each light reaches (#780).
//
// One invocation per light. It writes one work item per (light, slice)
// pair and bumps the rasterizer's instance count, so the raster pass
// that follows is dispatched from the GPU's own answer rather than from
// a count the CPU guessed a frame ago.
//
// Concatenated after `cluster_common.wgsl`.

@group(0) @binding(0) var<uniform> cluster_view: ClusterView;
@group(0) @binding(1) var<storage, read> cluster_lights: array<ClusterLight>;
@group(0) @binding(2) var<storage, read_write> cluster_draw: ClusterDraw;
@group(0) @binding(3) var<storage, read_write> cluster_slices: array<ZSlice>;

@compute @workgroup_size(64, 1, 1)
fn z_slice_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let light_index = id.x;
    if (light_index >= cluster_view.counts.x) {
        return;
    }

    let light = cluster_lights[light_index];
    // Directional lights reach everything, so a cell that listed one
    // would be saying nothing. They stay on the linear path in
    // `inti_shade`, where there are only ever a handful of them.
    var object_type = CLUSTER_TYPE_POINT;
    if (light.kind == 2u) {
        object_type = CLUSTER_TYPE_SPOT;
    } else if (light.kind != 1u) {
        return;
    }
    // A light with no reach lights nothing. Skipping it here keeps a
    // zero-radius sphere out of the projection maths below, where it
    // would still occupy the cell containing its centre.
    if (light.range <= 0.0) {
        return;
    }

    let sphere = cluster_light_sphere(light);
    let bounds = cluster_sphere_bounds(cluster_view, sphere.xyz, sphere.w);

    for (var z = u32(bounds.min.z); z <= u32(bounds.max.z); z = z + 1u) {
        write_slice(light_index, object_type, z);
    }
}

// Appends one work item, and reports rather than truncates when the list
// is full.
//
// 🔴 The counter is bumped before the capacity test, so the CPU learns
// the real number and can grow the buffer. Bumping it only on success
// would make an overflowing frame indistinguishable from one that fit —
// lights silently missing from cells, and nothing anywhere saying so.
fn write_slice(object_index: u32, object_type: u32, z_slice: u32) {
    let slot = atomicAdd(&cluster_draw.wanted, 1u);
    if (slot >= cluster_view.counts.y) {
        return;
    }
    cluster_slices[slot].object_index = object_index;
    cluster_slices[slot].object_type = object_type;
    cluster_slices[slot].z_slice = z_slice;
}

// Turns the uncapped count into draw arguments.
//
// One invocation, dispatched after the pass above has finished, because
// there is no barrier across workgroups: the clamp has to see the final
// total, and the only thing that guarantees that is a separate dispatch.
// Without it an overflowing frame would draw instances whose work items
// were never written — garbage cells, from a buffer that reports itself
// as fine.
@compute @workgroup_size(1, 1, 1)
fn finalize_main() {
    cluster_draw.instance_count = min(
        atomicLoad(&cluster_draw.wanted),
        cluster_view.counts.y,
    );
}
