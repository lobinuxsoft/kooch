// transparent_clip.wgsl — a clipped transparent material's insert (#452): what falls below its
// `alpha_clip` never takes a layer, so it is neither shaded nor composited.

fn transparent_keeps(frag: ForwardOut) -> bool {
    let surf = resolve_surface(frag.slot, frag.triangle, frag.position.xy);
    let shaded = surface(surface_input(surf, frag.position.xy));
    return shaded.alpha >= shaded.alpha_clip;
}
