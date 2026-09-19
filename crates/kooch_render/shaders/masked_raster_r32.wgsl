// masked_raster_r32.wgsl — the masked raster's R32 write, as `fs_vbuf_scene` makes it: the slot
// + 1, so 0 stays the background.

@fragment
fn fs_masked(frag: MaskedOut) -> @location(0) u32 {
    if (!masked_keeps(frag)) {
        discard;
    }
    return ((frag.slot + 1u) << 7u) | (frag.triangle & 0x7fu);
}
