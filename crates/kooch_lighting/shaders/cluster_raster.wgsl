// Passes 2 and 4 of 4: which lights actually touch each cell (#780).
//
// 🔴 This is a rasterizer, not a compute shader, and that is the part
// everyone's mental model of clustering gets wrong. The grid is WxHxD;
// the pass runs on a WxH viewport and draws each work item — one light
// in one Z slice — as a screen-aligned quad covering the cells that
// light's bounding sphere can reach. One fragment invocation is then
// exactly one (cell, light) pair, scheduled by the hardware that exists
// to schedule quads. Colour writes are off: the output is the storage
// buffers.
//
// It runs TWICE, with `{{CLUSTER_POPULATE}}` substituted false then
// true: once to count how many lights land in each cell, and once to
// write them now that each cell knows where its run starts.
//
// 🔴 Both runs must reach the same verdict for every pair. A count that
// disagrees with the populate either overflows a cell's run into its
// neighbour's or leaves a hole, and nothing in the pipeline can detect
// it — there is no compiler keeping the two in step, only the fact that
// they are literally the same source.
//
// Concatenated after `cluster_common.wgsl`.

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

    // The quad is the sphere's screen-space bounding box, which is
    // generous: it covers cells the sphere's projection touches but the
    // sphere itself never reaches, and every one of those would cost a
    // light in the shading loop. This is the test that throws them out.
    let aabb = cell_bounds(cell);
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

// The view-space bounds of one cell.
//
// The XY edges come from unprojecting the cell's screen rectangle at the
// near plane and following those rays out to the slice's near and far
// depths; the sides of a froxel are not axis-aligned, so the AABB of the
// eight resulting corners is what a cheap intersection test can use.
fn cell_bounds(cell: vec3<u32>) -> ClusterAabb {
    let near = cluster_view.z_factors.z;
    let far = cluster_view.z_factors.w;
    let slices = f32(cluster_view.dimensions.z);

    let p_min = vec2<f32>(cell.xy) * cluster_view.viewport.zw;
    let p_max = p_min + cluster_view.viewport.zw;

    let ray_min = screen_to_view(p_min).xyz;
    let ray_max = screen_to_view(p_max).xyz;

    // The slice boundaries, from the same logarithmic distribution
    // `cluster_z_slice` inverts. Slice 0 starts at the eye rather than
    // at `near`, because that is where everything nearer than the first
    // slice ends up.
    let ratio = far / near;
    let z = f32(cell.z);
    var slice_near = 0.0;
    if (z != 0.0) {
        slice_near = -near * pow(ratio, (z - 1.0) / max(slices - 1.0, 1.0));
    }
    let slice_far = -near * pow(ratio, z / max(slices - 1.0, 1.0));

    let a = ray_to_depth(ray_min, slice_near);
    let b = ray_to_depth(ray_min, slice_far);
    let c = ray_to_depth(ray_max, slice_near);
    let d = ray_to_depth(ray_max, slice_far);

    return ClusterAabb(min(min(a, b), min(c, d)), max(max(a, b), max(c, d)));
}

// A pixel position on the near plane, in view space.
fn screen_to_view(screen: vec2<f32>) -> vec4<f32> {
    let uv = screen / cluster_view.viewport.xy;
    // NDC z of 1.0 is the near plane: the projection is reversed-Z
    // (ADR 0002), so near is far and far is zero.
    let clip = vec4<f32>(uv.x * 2.0 - 1.0, (1.0 - uv.y) * 2.0 - 1.0, 1.0, 1.0);
    let view = cluster_view.view_from_clip * clip;
    return view / view.w;
}

// Where the ray from the eye through `p` crosses a plane of constant z.
fn ray_to_depth(p: vec3<f32>, z: f32) -> vec3<f32> {
    // A ray parallel to the plane cannot cross it. `p.z` is the near
    // plane's depth, never zero, but the guard costs one compare and the
    // alternative is a NaN that propagates into the cell's bounds.
    if (abs(p.z) < 1e-9) {
        return p;
    }
    return p * (z / p.z);
}

fn sphere_hits_aabb(
    sphere_center: vec3<f32>,
    sphere_radius: f32,
    aabb_center: vec3<f32>,
    aabb_half: vec3<f32>,
) -> bool {
    let delta = max(vec3<f32>(0.0), abs(aabb_center - sphere_center) - aabb_half);
    return dot(delta, delta) <= sphere_radius * sphere_radius;
}

// Cone against the cell's bounding sphere — Bart Wronski's test.
//
// Three ways a cone can miss: the cell is off to the side of the cone's
// angle, past its tip, or behind its apex.
fn cone_misses_cell(
    light_index: u32,
    cell_center: vec3<f32>,
    cell_radius: f32,
    sphere_center: vec3<f32>,
) -> bool {
    let light = cluster_lights[light_index];
    // 🔴 The axis points BACK along the light, not along it. The offset
    // below runs from the cell to the light, so a cell inside the cone
    // has to come out with a POSITIVE `along` — and with the direction
    // the light shines in, every such cell reads as negative and is
    // thrown out by `back_miss`. The symptom is a scene lit only where
    // the light is not pointing.
    //
    // ⚠️ Bevy does the same flip, and their variable for it is named
    // `world_light_direction` right after being assigned the negation of
    // what they called the reverse. Reading the name rather than the
    // maths is how this was wrong the first time.
    let axis = normalize((cluster_view.view_from_world * vec4<f32>(-light.direction, 0.0)).xyz);

    // The cone's half-angle, recovered from the falloff MAD the shading
    // model already stores: `saturate(cos * scale + offset)` reaches
    // zero at the outer angle, so `cos_outer = -offset / scale`.
    //
    // ⚠️ Bevy sends a tangent for this and rebuilds the direction from
    // two components — both are consequences of their light record
    // having no room, not of the maths. Ours carries the direction and
    // the MAD already.
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

// Populate pass: claim the next slot of this cell's run for this type
// and write the index there.
//
// The run's layout is the type order itself — points, then spots, then
// probes, volumes, decals — so a type's base is the cell's offset plus
// the counts of every type before it. That is what lets the shading loop
// walk one type as a plain range with no test inside it.
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
