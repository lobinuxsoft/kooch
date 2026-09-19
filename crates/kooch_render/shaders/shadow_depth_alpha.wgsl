// shadow_depth_alpha.wgsl — the classic shadow rasters' variant while a transparent material casts
// (#1224): the same corners, and a fragment that drops what the baked coverage does not reach.
// Opaque-only frames keep the fragment-less pipeline.

@vertex
fn vs_shadow_alpha(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> ShadowCorner {
    return shadow_corner(vertex_index, instance_index);
}

@fragment
fn fs_shadow_alpha(in: ShadowCorner) {
    if !shadow_alpha_keeps(in.material, in.uv, in.clip.xy) {
        discard;
    }
}
