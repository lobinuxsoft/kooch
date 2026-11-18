// Planet generation (#313 step 3): one workgroup per block, each sample evaluated by the
// generator's `fn planet(input: PlanetInput) -> f32` and stored in voxels of the block's level.
//
// Concatenation order (built by `GeneratePass::new`):
//   PLANET_INPUT_WGSL (the struct the generator reads)
// + generator fragment (defines `fn planet`)
// + this file

override TILES_X: u32 = 32u;
override TILES_Y: u32 = 1u;
override TILE_DIM: u32 = 17u;

const WORKGROUP: u32 = 256u;
// Voxels kept either side of the surface; beyond it a meshing pass needs only the sign, and f16
// would lose a far value anyway.
const BAND: f32 = 64.0;

struct Job {
    // Block corner from the planet's centre, and the edge of one voxel at its level.
    origin: vec3<f32>,
    voxel: f32,
    radius: f32,
    // |origin|² − radius², taken in f64 on the CPU: what keeps the altitude exact far out.
    excess: f32,
    slot: u32,
    planet: u32,
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
        let local = vec3<f32>(v) * job.voxel;
        let position = job.origin + local;
        let distance = length(position);
        // |o + l| − R = (|o|² − R² + 2o·l + l²) / (|o + l| + R): no f32 radius is subtracted.
        let altitude = (job.excess + 2.0 * dot(job.origin, local) + dot(local, local))
            / (distance + job.radius);
        var input: PlanetInput;
        input.position = position;
        input.altitude = altitude;
        input.direction = position / max(distance, 1e-6);
        input.radius = job.radius;
        input.planet = job.planet;
        let sdf = clamp(planet(input) / job.voxel, -BAND, BAND);
        textureStore(atlas, corner + vec3<i32>(v), vec4<f32>(sdf, 0.0, 0.0, 0.0));
        i = i + WORKGROUP;
    }
}
