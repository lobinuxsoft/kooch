use super::{DOWNSAMPLE_WGSL, DOWNSAMPLE_WORKGROUP_SIZE};

/// Host constant, shader `const` and `@workgroup_size` must agree, and nothing fails to compile
/// when they don't: a mismatched stride reads some voxels twice and skips others.
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
