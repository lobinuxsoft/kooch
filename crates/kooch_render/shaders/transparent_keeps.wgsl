// transparent_keeps.wgsl — the shared insert's: a material without a clip keeps every fragment.
// Also the depth the other frames get from the contact march, which this one does not compose.

@group(0) @binding(4) var depth_prepass_texture: texture_depth_2d;

fn transparent_keeps(frag: ForwardOut) -> bool {
    return true;
}
