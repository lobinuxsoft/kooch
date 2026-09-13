// Passes 2 and 4 (#780): a rasterizer, one quad per (light, slice), so a fragment is one (cell,
// light) pair. 🔴 Runs twice via `{{CLUSTER_POPULATE}}` — count, then write — and only sharing this
// source keeps both verdicts equal.

const POPULATE: bool = {{CLUSTER_POPULATE}};

@group(0) @binding(0) var<uniform> cluster_view: ClusterView;
@group(0) @binding(1) var<storage, read> cluster_lights: array<ClusterLight>;
@group(0) @binding(2) var<storage, read> cluster_slices: array<ZSlice>;
@group(0) @binding(3) var<storage, read_write> cluster_cells: array<ClusterCell>;
@group(0) @binding(4) var<storage, read_write> cluster_scratch: array<ClusterCell>;
@group(0) @binding(5) var<storage, read_write> cluster_indices: array<u32>;

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) slice_index: u32,
    // The light's bounding sphere in view space — computed once per
    // quad rather than once per cell.
    @location(1) @interpolate(flat) sphere_center: vec3<f32>,
    @location(2) @interpolate(flat) sphere_radius: f32,
}

@vertex
fn vertex_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> Varyings {
    let work = cluster_slices[instance_index];
    let light = cluster_lights[work.object_index];
    let sphere = cluster_light_sphere(light);

    let bounds = cluster_sphere_bounds(cluster_view, sphere.xyz, sphere.w);
    // Inclusive cells to an exclusive quad edge: a light covering only
    // cell 3 spans from 3 to 4.
    let cell_min = vec2<u32>(bounds.min.xy);
    let cell_max = vec2<u32>(bounds.max.xy) + vec2<u32>(1u);

    let corner = quad_corner(vertex_index);
    let cell = vec2<f32>(select(cell_min, cell_max, corner == vec2<u32>(1u)));
    let uv = cell / vec2<f32>(cluster_view.dimensions.xy);
    // UV to NDC, with Y flipped: cell (0,0) is the top-left of the grid
    // the same way pixel (0,0) is the top-left of the screen.
    let ndc = mix(vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), uv);

    let center = (cluster_view.view_from_world * vec4<f32>(sphere.xyz, 1.0)).xyz;
    let scale = max(
        cluster_view.view_scale.x,
        max(cluster_view.view_scale.y, cluster_view.view_scale.z),
    );

    return Varyings(vec4<f32>(ndc, 0.0, 1.0), instance_index, center, sphere.w * scale);
}

// The six vertices of a quad as unit corners.
fn quad_corner(vertex_index: u32) -> vec2<u32> {
    let x = select(0u, 1u, vertex_index == 1u || vertex_index == 4u || vertex_index == 5u);
    let y = select(0u, 1u, vertex_index == 2u || vertex_index == 3u || vertex_index == 5u);
    return vec2<u32>(x, y);
}

@fragment
fn fragment_main(varyings: Varyings) -> @location(0) vec4<f32> {
    let work = cluster_slices[varyings.slice_index];
    let cell = vec3<u32>(vec2<u32>(floor(varyings.position.xy)), work.z_slice);

    // The quad is the sphere's screen bounds, generous; this test drops cells the sphere never
    // reaches, each of which would cost a light when shading.
    let aabb = cluster_cell_bounds(cluster_view, cell);
    let center = (aabb.max + aabb.min) * 0.5;
    let half = (aabb.max - aabb.min) * 0.5;
    if (!sphere_hits_aabb(varyings.sphere_center, varyings.sphere_radius, center, half)) {
        return vec4<f32>(0.0);
    }

    // A spot light's sphere is its range; the cone inside it is usually
    // a small fraction of that volume, so this second test is where most
    // of a spot-lit scene's savings come from.
    if (work.object_type == CLUSTER_TYPE_SPOT
        && cone_misses_cell(work.object_index, center, length(half), varyings.sphere_center)) {
        return vec4<f32>(0.0);
    }

    let index = cluster_index(cell, cluster_view.dimensions);
    if (POPULATE) {
        write_index(index, work.object_type, work.object_index);
    } else {
        count_object(index, work.object_type);
    }
    return vec4<f32>(0.0);
}

// `cluster_cell_bounds`, `view_at_screen` and `ray_at_depth` now live
// in `cluster_common.wgsl`, because the page marking needs them too.

fn sphere_hits_aabb(
    sphere_center: vec3<f32>,
    sphere_radius: f32,
    aabb_center: vec3<f32>,
    aabb_half: vec3<f32>,
) -> bool {
    let delta = max(vec3<f32>(0.0), abs(aabb_center - sphere_center) - aabb_half);
    return dot(delta, delta) <= sphere_radius * sphere_radius;
}

// Cone against the cell's bounding sphere (Bart Wronski): it misses off to the side, past the tip,
// or behind the apex.
fn cone_misses_cell(
    light_index: u32,
    cell_center: vec3<f32>,
    cell_radius: f32,
    sphere_center: vec3<f32>,
) -> bool {
    let light = cluster_lights[light_index];
    // 🔴 The axis points back along the light: the offset runs cell → light, so a lit cell must come
    // out positive. Bevy's `world_light_direction` is the negated value, despite the name — reading
    // the name made this wrong once.
    let axis = normalize((cluster_view.view_from_world * vec4<f32>(-light.direction, 0.0)).xyz);

    // Half-angle recovered from the stored MAD: `saturate(cos * scale + offset)` is zero at the
    // outer angle, so `cos_outer = -offset / scale`.
    let cos_outer = clamp(-light.spot_offset / max(light.spot_scale, 1e-6), -1.0, 1.0);
    let sin_outer = sqrt(max(1.0 - cos_outer * cos_outer, 0.0));

    let offset = sphere_center - cell_center;
    let along = dot(offset, axis);
    let across = sqrt(max(dot(offset, offset) - along * along, 0.0));

    let closest = cos_outer * across - along * sin_outer;
    let angle_miss = closest > cell_radius;
    let front_miss = along > cell_radius + light.range;
    let back_miss = along < -cell_radius;
    return angle_miss || front_miss || back_miss;
}

// Counting pass: one more object of this type in this cell.
fn count_object(cell: u32, object_type: u32) {
    switch (object_type) {
        case 0u: { atomicAdd(&cluster_cells[cell].point_count, 1u); }
        case 1u: { atomicAdd(&cluster_cells[cell].spot_count, 1u); }
        case 2u: { atomicAdd(&cluster_cells[cell].probe_count, 1u); }
        case 3u: { atomicAdd(&cluster_cells[cell].volume_count, 1u); }
        case 4u: { atomicAdd(&cluster_cells[cell].decal_count, 1u); }
        default: {}
    }
}

// Populate: claims the next slot of this type's range. The run is laid out in type order, so the
// shading loop walks one type as a plain range.
fn write_index(cell: u32, object_type: u32, object_index: u32) {
    let base = atomicLoad(&cluster_cells[cell].offset);
    var slot = 0xffffffffu;
    switch (object_type) {
        case 0u: {
            slot = base + atomicAdd(&cluster_scratch[cell].point_count, 1u);
        }
        case 1u: {
            slot = base + atomicLoad(&cluster_cells[cell].point_count)
                + atomicAdd(&cluster_scratch[cell].spot_count, 1u);
        }
        case 2u: {
            slot = base + atomicLoad(&cluster_cells[cell].point_count)
                + atomicLoad(&cluster_cells[cell].spot_count)
                + atomicAdd(&cluster_scratch[cell].probe_count, 1u);
        }
        case 3u: {
            slot = base + atomicLoad(&cluster_cells[cell].point_count)
                + atomicLoad(&cluster_cells[cell].spot_count)
                + atomicLoad(&cluster_cells[cell].probe_count)
                + atomicAdd(&cluster_scratch[cell].volume_count, 1u);
        }
        case 4u: {
            slot = base + atomicLoad(&cluster_cells[cell].point_count)
                + atomicLoad(&cluster_cells[cell].spot_count)
                + atomicLoad(&cluster_cells[cell].probe_count)
                + atomicLoad(&cluster_cells[cell].volume_count)
                + atomicAdd(&cluster_scratch[cell].decal_count, 1u);
        }
        default: {}
    }
    if (slot < cluster_view.counts.z) {
        cluster_indices[slot] = object_index;
    }
}
