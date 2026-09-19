// transparent_shade.wgsl — one material's layers lit in place (#452). Each material's pass shades
// the keys that name it and leaves every other one for its own.

@group(0) @binding(5) var<storage, read_write> layers: array<u64>;

@compute @workgroup_size(8, 8)
fn cs_shade(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= screen.size)) {
        return;
    }
    let frag_coord = vec2<f32>(id.xy) + 0.5;
    for (var layer = 0u; layer < LAYERS; layer += 1u) {
        let at = layer_at(id.xy, screen.size.x, layer);
        let key = layers[at];
        if (key == 0lu) {
            return;
        }
        if (is_shaded(key)) {
            continue;
        }
        let slot = key_slot(key);
        if (instances[visible_meshlets[slot] >> 16u].material_id != screen.material_id) {
            continue;
        }
        layers[at] = pack_shaded(transparent_lit(slot, key_triangle(key), frag_coord));
    }
}
