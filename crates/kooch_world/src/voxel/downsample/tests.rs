use super::{DOWNSAMPLE_WGSL, DOWNSAMPLE_WORKGROUP_SIZE};

/// The workgroup size is written in three places and none of them fails
/// to compile when they disagree.
///
/// The host constant sizes the dispatch, the shader's `const` is the
/// stride of the grid-stride loop at `sparse_downsample.wgsl:154`, and
/// `@workgroup_size` is how many threads actually run. Divergence
/// between the last two makes the loop step by a different amount than
/// there are threads, so the pass reads some voxels twice and skips
/// others — a cascade that is quietly wrong rather than a build that
/// stops.
///
/// `POPULATE_WORKGROUP_SIZE` has had this test since it was written;
/// this is the same one for its twin.
#[test]
fn downsample_workgroup_size_agrees() {
    assert!(
        DOWNSAMPLE_WGSL.contains(&format!(
            "DOWNSAMPLE_WORKGROUP_SIZE: u32 = {DOWNSAMPLE_WORKGROUP_SIZE}u",
        )),
        "the shader's DOWNSAMPLE_WORKGROUP_SIZE has diverged from the host's",
    );
    assert!(
        DOWNSAMPLE_WGSL.contains(&format!("@workgroup_size({DOWNSAMPLE_WORKGROUP_SIZE})")),
        "@workgroup_size has diverged from DOWNSAMPLE_WORKGROUP_SIZE — the \
         grid-stride loop would step by a different count than there are threads",
    );
}
