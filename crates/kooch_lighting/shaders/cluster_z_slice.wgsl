// Pass 1 of 4 (#780): one work item per (light, slice) and the raster's instance count, so the
// raster dispatches from the GPU's answer, not a stale CPU guess. After `cluster_common.wgsl`.

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

// Appends a work item. 🔴 The counter rises before the capacity test, so an overflowing frame is
// visible to the CPU instead of silently dropping lights.
fn write_slice(object_index: u32, object_type: u32, z_slice: u32) {
    let slot = atomicAdd(&cluster_draw.wanted, 1u);
    if (slot >= cluster_view.counts.y) {
        return;
    }
    cluster_slices[slot].object_index = object_index;
    cluster_slices[slot].object_type = object_type;
    cluster_slices[slot].z_slice = z_slice;
}

// Uncapped count → draw arguments, in its own dispatch: no barrier crosses workgroups, and the
// clamp must see the final total or the raster draws unwritten items.
@compute @workgroup_size(1, 1, 1)
fn finalize_main() {
    cluster_draw.instance_count = min(
        atomicLoad(&cluster_draw.wanted),
        cluster_view.counts.y,
    );
}
