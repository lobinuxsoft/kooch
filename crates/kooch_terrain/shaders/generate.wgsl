// Planet generation (#313 step 3): one workgroup per block, each sample evaluated by the
// generator's `fn planet(input: PlanetInput) -> f32` and stored in voxels of the block's level.
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
}

@group(0) @binding(0) var<storage, read> jobs: array<Job>;
@group(0) @binding(1) var atlas: texture_storage_3d<r16float, write>;

@compute @workgroup_size(256)
fn generate_main(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
) {
    if (wid.x >= arrayLength(&jobs)) {
        return;
    }
    let job = jobs[wid.x];
    let base = job.index & vec3<i32>(-ANCHOR);
    let tile = vec3<u32>(
        job.slot % TILES_X,
        (job.slot / TILES_X) % TILES_Y,
        job.slot / (TILES_X * TILES_Y),
    );
    let corner = vec3<i32>(tile * TILE_DIM);
    let voxels = TILE_DIM * TILE_DIM * TILE_DIM;
    var i: u32 = lid.x;
    loop {
        if (i >= voxels) {
            break;
        }
        let v = vec3<u32>(i % TILE_DIM, (i / TILE_DIM) % TILE_DIM, i / (TILE_DIM * TILE_DIM));
        // From the grid index, and the altitude from an anchor fixed on the grid, not from the
        // tile: a neighbour reaches a shared sample through the same integers, so both store the
        // same bits and their meshes meet (#1933, #393).
        let grid = job.index + vec3<i32>(v);
        let position = vec3<f32>(grid) * job.voxel;
        let distance = length(position);
        let anchor = grid & vec3<i32>(-ANCHOR);
        let k = vec3<u32>((anchor - base) / ANCHOR);
        let origin = vec3<f32>(anchor) * job.voxel;
        let local = vec3<f32>(grid - anchor) * job.voxel;
        // |o + l| − R = (|o|² − R² + 2o·l + l²) / (|o + l| + R): no f32 radius is subtracted.
        let excess = jobs[wid.x].excess[k.x + 2u * k.y + 4u * k.z];
        let altitude = (excess + 2.0 * dot(origin, local) + dot(local, local))
            / (distance + job.radius);
        var input: PlanetInput;
        input.position = position;
        input.altitude = altitude;
        input.direction = position / max(distance, 1e-6);
        // `y` is an integer times the voxel, exact without anchors.
        if (job.plane == 1u) {
            input.altitude = position.y;
            input.direction = vec3<f32>(0.0, 1.0, 0.0);
        }
        input.radius = job.radius;
        input.planet = job.planet;
        let sdf = clamp(planet(input) / job.voxel, -BAND, BAND);
        textureStore(atlas, corner + vec3<i32>(v), vec4<f32>(sdf, 0.0, 0.0, 0.0));
        i = i + WORKGROUP;
    }
}
