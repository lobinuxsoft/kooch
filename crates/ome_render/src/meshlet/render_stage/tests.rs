use super::*;

#[test]
fn config_defaults_are_non_zero() {
    let cfg = MeshletRenderStageConfig::default();
    assert!(cfg.size.0 > 0 && cfg.size.1 > 0);
    assert!(cfg.instance_capacity > 0);
    assert!(cfg.meshlet_capacity > 0);
    assert!(!cfg.materials.is_empty());
}

#[test]
fn stats_default_is_zero() {
    let s = MeshletRenderStats::default();
    assert_eq!(s.instances_uploaded, 0);
    assert_eq!(s.cull_threads, 0);
}

#[test]
fn hi_z_byte_size_matches_summed_mips() {
    // Pure-CPU verification of the pyramid-byte arithmetic that
    // set_vram_tracker / resize rely on. R32Float = 4 bpp summed
    // over the mip chain — for a square power-of-two pyramid this
    // converges to base * 4/3 (geometric series factor 1/4).
    // 64x64 → 7 mips, exact total = (4096+1024+256+64+16+4+1) * 4 = 21844.
    let expected = (4096 + 1024 + 256 + 64 + 16 + 4 + 1) * 4u64;
    let mip_count = crate::hi_z::mip_count_for(64, 64);
    let mut total: u64 = 0;
    for level in 0..mip_count {
        let (w, h) = crate::hi_z::mip_size(64, 64, level);
        total += (w as u64) * (h as u64) * 4;
    }
    assert_eq!(total, expected);
}
