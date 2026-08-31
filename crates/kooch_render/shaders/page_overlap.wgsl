// "Does this rectangle touch any page being drawn this frame?", in
// constant time (#1022).
//
// Pure functions over the page pyramid `page_pyramid.wgsl` builds:
// no bindings of its own, so the caller passes the texture it already
// has bound. The expansion and the tests both include this and are
// therefore asking the same question, which is the only way a test of
// it means anything.

/// The LOWEST pyramid mip at which `rect` spans at most two texels per
/// axis — the coarsest read the four loads below can answer it with.
///
/// # 🔴 Not a function of the rectangle's size
///
/// The obvious closed forms are all wrong, and wrong in the expensive
/// direction. The highest differing bit of the two ends says `1` for a
/// span of `[1, 2]`, which mip 0 already answers: 1 and 2 are adjacent
/// texels, and four explicit loads do not need the pair to be aligned
/// to a power-of-two block the way a hardware gather would.
///
/// Whether a span collapses depends on WHERE it sits, not only on how
/// long it is: `[1, 2]` fits at mip 0 and `[1, 3]` does not, at the
/// same length as `[2, 3]`, which does. So this walks the chain and
/// takes the first mip that holds, which is at most eight iterations of
/// two shifts — nothing beside the texture read it saves.
///
/// A mip too high is still SAFE: bigger blocks over-report, and an
/// over-report costs one pair tested and discarded. It is only the
/// wrong answer to the question "how much work does this cost".
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
///
/// `rect` is inclusive, in PAGES, and already clamped to the level's
/// grid — a rectangle running off the edge would wrap into the far side
/// of the world and report a page that is nowhere near the caster.
///
/// ⚠️ Conservative by construction, and it has to be. At the chosen mip
/// a texel stands for a whole block, so a rectangle that merely touches
/// the block of a listed page answers `true`. That costs a caster
/// tested against a page it turns out to miss; the opposite error would
/// drop geometry that does cast, and nothing downstream could tell.
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
