use super::{ClassicAlloc, classic_shadow_alloc};
use crate::shadow::ShadowSettings;

/// #945: the pages replace every classic reader, so the classic pass
/// holds a token allocation — not zero, because the shading's bind
/// group needs live views — and gets its real one back when the pages
/// turn off.
#[test]
fn the_pages_shrink_the_classic_alloc() {
    let mut settings = ShadowSettings::default();
    settings.virtual_pages = false;
    let full = classic_shadow_alloc(&settings);
    assert_eq!(full.texels, settings.clamped_texels());
    assert_eq!(full.cube_size, crate::shadow::DEFAULT_CUBE_SIZE);

    settings.virtual_pages = true;
    let token = classic_shadow_alloc(&settings);
    assert_eq!((token.texels, token.cube_size, token.cubes), (256, 16, 1));

    // The two must differ, whatever the author set: equality is what
    // the resize-release door compares, and a collision would leave the
    // full allocation standing under the pages.
    assert_ne!(
        full, token,
        "the token allocation collides with the full one"
    );
    // And neither is the zeroed sentinel, or the release would read as
    // "already holding it".
    assert_ne!(full, ClassicAlloc::default());
    assert_ne!(token, ClassicAlloc::default());
}

/// The collision the tuple exists to prevent: an author who set the
/// cascades to the clamp floor still swaps allocations on toggle,
/// because the cube half of the key differs.
#[test]
fn a_floor_sized_atlas_still_swaps() {
    let mut settings = ShadowSettings::default();
    settings.cascade_texels = 256;
    settings.virtual_pages = false;
    let full = classic_shadow_alloc(&settings);
    settings.virtual_pages = true;
    let token = classic_shadow_alloc(&settings);
    assert_eq!(full.texels, token.texels, "the premise: both at the floor");
    assert_ne!(
        full, token,
        "a bare texel count would have compared equal here"
    );
}
