// transparent_insert.wgsl — every transparent fragment offered to its pixel's four layers (#452).
// One pipeline for every material: nothing is shaded here, only kept or pushed out.

@group(0) @binding(4) var depth_prepass_texture: texture_depth_2d;
@group(0) @binding(5) var<storage, read_write> layers: array<atomic<u64>>;
@group(0) @binding(6) var<storage, read_write> overflow: array<atomic<u32>>;

@fragment
fn fs_insert(in: ForwardOut) {
    if (opaque_covers(in.position)) {
        return;
    }
    let pixel = vec2<u32>(in.position.xy);
    var key = layer_key(in.position.z, in.slot, in.triangle);
    // Insertion from the nearest layer down: each keeps the larger (nearer) of what it held and
    // what arrived, and the smaller moves on. What leaves the last layer is the tail's.
    for (var layer = 0u; layer < LAYERS; layer += 1u) {
        let held = atomicMax(&layers[layer_at(pixel, screen.size.x, layer)], key);
        key = min(held, key);
        if (key == 0lu) {
            return;
        }
    }
    atomicMax(&overflow[0], 1u);
}
