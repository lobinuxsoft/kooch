// Planet generation (#313 step 3): one workgroup per block, each sample evaluated by the
// generator's `fn planet(input: PlanetInput) -> f32` and stored in voxels of the block's level.
// Then `material_main` gives each sample its layers from `fn planet_material` (#1951).
//
// Concatenation order (built by `GeneratePass::new`):
//   PLANET_INPUT_WGSL (the struct the generator reads)
// + generator fragment (defines `fn planet`)
// + this file
// after the atlas sizes `TILES_X`, `TILES_Y` and `TILE_DIM` as consts. Not overrides: naga's override
// pass drops a generator's composed call argument out of scope ("Function 'planet' is invalid").

const WORKGROUP: u32 = 256u;
// Voxels kept either side of the surface; beyond it a meshing pass needs only the sign, and f16
// would lose a far value anyway.
const BAND: f32 = 64.0;
// Grid period of the altitude anchors. A tile spans 18 samples, so it meets at most two per axis.
const ANCHOR: i32 = 32;

struct Job {
    // The tile's first sample on the planet's grid, in voxels of this level.
    index: vec3<i32>,
    // The edge of one voxel at the tile's level.
    voxel: f32,
    radius: f32,
    slot: u32,
    planet: u32,
    // 1 for a plane: altitude is `y`, up is +Y (#1950).
    plane: u32,
    // |anchor|² − radius² for the eight anchors the tile meets, x fastest, taken in f64 on the CPU:
    // what keeps the altitude exact far out.
    excess: array<f32, 8>,
    // The finest level's voxel: the step the material measures slope with, at every level (#1951).
    fine: f32,
}

@group(0) @binding(0) var<storage, read> jobs: array<Job>;
@group(0) @binding(1) var atlas: texture_storage_3d<r16float, write>;
// The material pass reads what the distance pass wrote, and writes each sample's layers (#1951).
@group(0) @binding(2) var samples: texture_3d<f32>;
@group(0) @binding(3) var materials: texture_storage_3d<r32uint, write>;

// Job `w`'s sample `v` as the generator reads it.
fn input_at(w: u32, v: vec3<u32>) -> PlanetInput {
    return input_near(w, v, vec3<f32>(0.0));
}

// The same, `delta` metres away: through the sample's anchor, so it stays exact far out.
fn input_near(w: u32, v: vec3<u32>, delta: vec3<f32>) -> PlanetInput {
    let job = jobs[w];
    let base = job.index & vec3<i32>(-ANCHOR);
    // From the grid index, and the altitude from an anchor fixed on the grid, not from the
    // tile: a neighbour reaches a shared sample through the same integers, so both store the
    // same bits and their meshes meet (#1933, #393).
    let grid = job.index + vec3<i32>(v);
    let position = vec3<f32>(grid) * job.voxel + delta;
    let distance = length(position);
    let anchor = grid & vec3<i32>(-ANCHOR);
    let k = vec3<u32>((anchor - base) / ANCHOR);
    let origin = vec3<f32>(anchor) * job.voxel;
    let local = vec3<f32>(grid - anchor) * job.voxel + delta;
    // |o + l| − R = (|o|² − R² + 2o·l + l²) / (|o + l| + R): no f32 radius is subtracted.
    let excess = jobs[w].excess[k.x + 2u * k.y + 4u * k.z];
    var input: PlanetInput;
    input.position = position;
    input.altitude = (excess + 2.0 * dot(origin, local) + dot(local, local))
        / (distance + job.radius);
    input.direction = position / max(distance, 1e-6);
    // `y` is an integer times the voxel, exact without anchors.
    if (job.plane == 1u) {
        input.altitude = position.y;
        input.direction = vec3<f32>(0.0, 1.0, 0.0);
    }
    input.radius = job.radius;
    input.planet = job.planet;
    return input;
}

fn tile_corner(slot: u32) -> vec3<i32> {
    let tile = vec3<u32>(slot % TILES_X, (slot / TILES_X) % TILES_Y, slot / (TILES_X * TILES_Y));
    return vec3<i32>(tile * TILE_DIM);
}

fn voxel_of(i: u32) -> vec3<u32> {
    return vec3<u32>(i % TILE_DIM, (i / TILE_DIM) % TILE_DIM, i / (TILE_DIM * TILE_DIM));
}

@compute @workgroup_size(256)
fn generate_main(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
) {
    if (wid.x >= arrayLength(&jobs)) {
        return;
    }
    let job = jobs[wid.x];
    let corner = tile_corner(job.slot);
    for (var i = lid.x; i < TILE_DIM * TILE_DIM * TILE_DIM; i += WORKGROUP) {
        let v = voxel_of(i);
        let sdf = clamp(planet(input_at(wid.x, v)) / job.voxel, -BAND, BAND);
        textureStore(atlas, corner + vec3<i32>(v), vec4<f32>(sdf, 0.0, 0.0, 0.0));
    }
}

// Samples further from the surface than this, in voxels, give no vertex its material.
const NEAR: f32 = 2.0;

// The generator's gradient `delta` from sample `v`, over the finest voxel: the slope a level shows
// would smooth with its voxel, and the ground would change rock for grass as the camera moves.
fn gradient(w: u32, v: vec3<u32>, delta: vec3<f32>) -> vec3<f32> {
    let h = jobs[w].fine;
    var out = vec3<f32>(0.0);
    for (var axis = 0u; axis < 3u; axis++) {
        var e = vec3<f32>(0.0);
        e[axis] = h;
        out[axis] = planet(input_near(w, v, delta + e)) - planet(input_near(w, v, delta - e));
    }
    return out;
}

// The four heaviest of a sample's layer weights, packed as `MATERIAL_FORMAT` in atlas.rs says:
// ids 5 bits each, then the first three weights over their sum in 4 bits each. None weighs: layer 0.
fn strongest(weights: array<f32, LAYERS>) -> u32 {
    var all = weights;
    var ids = vec4<u32>(0u);
    var top = vec4<f32>(0.0);
    for (var l = 0u; l < LAYERS; l++) {
        let w = all[l];
        if (w <= top.w) {
            continue;
        }
        // Insertion into a sorted four: each place takes the one above it until `w` fits.
        top.w = top.z;
        ids.w = ids.z;
        if (w <= top.z) {
            top.z = w;
            ids.z = l;
            continue;
        }
        top.z = top.y;
        ids.z = ids.y;
        if (w <= top.y) {
            top.y = w;
            ids.y = l;
            continue;
        }
        top.y = top.x;
        ids.y = ids.x;
        top.x = w;
        ids.x = l;
    }
    let sum = top.x + top.y + top.z + top.w;
    if (sum <= 0.0) {
        return 15u << 20u;
    }
    let share = vec3<u32>(round(top.xyz / sum * 15.0));
    return ids.x | (ids.y << 5u) | (ids.z << 10u) | (ids.w << 15u)
        | (share.x << 20u) | (share.y << 24u) | (share.z << 28u);
}

@compute @workgroup_size(256)
fn material_main(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
) {
    if (wid.x >= arrayLength(&jobs)) {
        return;
    }
    let corner = tile_corner(jobs[wid.x].slot);
    for (var i = lid.x; i < TILE_DIM * TILE_DIM * TILE_DIM; i += WORKGROUP) {
        let v = voxel_of(i);
        var packed = 0u;
        if (abs(textureLoad(samples, corner + vec3<i32>(v), 0).x) < NEAR) {
            // Measured on the surface the sample stands for, not at the sample: on a coarse level
            // that is metres off, and a height rule would paint it as a peak.
            let toward = gradient(wid.x, v, vec3<f32>(0.0));
            let size = length(toward);
            var input = input_at(wid.x, v);
            if (size > 1e-6) {
                let normal = toward / size;
                let up = input.direction;
                input = input_near(wid.x, v, -normal * planet(input));
                input.slope = 1.0 - dot(normal, up);
            }
            packed = strongest(planet_material(input));
        }
        textureStore(materials, corner + vec3<i32>(v), vec4<u32>(packed));
    }
}
