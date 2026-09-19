// masked_raster_r64.wgsl — the masked raster's R64 write, as `fs_vbuf64_scene` makes it.

@group(0) @binding(5) var vbuf64: texture_storage_2d<r64uint, atomic>;

@fragment
fn fs_masked(frag: MaskedOut) {
    if (!masked_keeps(frag)) {
        discard;
    }
    let depth_bits = bitcast<u32>(frag.position.z);
    let id = (frag.slot << 7u) | (frag.triangle & 0x7fu);
    textureAtomicMax(vbuf64, vec2<u32>(frag.position.xy), (u64(depth_bits) << 32u) | u64(id));
}
