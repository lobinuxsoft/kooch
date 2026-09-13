// "Does this rectangle touch any page being drawn this frame?", in constant time (#1022).

/// The LOWEST pyramid mip at which `rect` spans at most two texels per axis — the coarsest read the
/// four loads below can answer it with.
fn overlap_mip(rect: vec4<u32>, mips: u32) -> u32 {
    for (var mip = 0u; mip + 1u < mips; mip = mip + 1u) {
        let wide = (rect.z >> mip) - (rect.x >> mip);
        let tall = (rect.w >> mip) - (rect.y >> mip);
        if wide <= 1u && tall <= 1u {
            return mip;
        }
    }
    return mips - 1u;
}

/// `true` when any page under `rect` is being drawn this frame.
fn overlaps_any_page(
    pyramid: texture_2d_array<u32>,
    rect: vec4<u32>,
    layer: u32,
    mips: u32,
) -> bool {
    if rect.z < rect.x || rect.w < rect.y {
        return false;
    }
    let mip = overlap_mip(rect, mips);
    let low = vec2<i32>(vec2<u32>(rect.x >> mip, rect.y >> mip));
    let high = vec2<i32>(vec2<u32>(rect.z >> mip, rect.w >> mip));
    let at = i32(layer);
    let m = i32(mip);
    // Four reads cover a 2x2 footprint exactly, and repeat harmlessly
    // when the rectangle is one texel wide or tall.
    var any = textureLoad(pyramid, low, at, m).x;
    any = any | textureLoad(pyramid, vec2<i32>(high.x, low.y), at, m).x;
    any = any | textureLoad(pyramid, vec2<i32>(low.x, high.y), at, m).x;
    any = any | textureLoad(pyramid, high, at, m).x;
    return any != 0u;
}
