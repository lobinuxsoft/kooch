use super::*;

/// The same guard the raster has, on the mirror that has existed
/// longest.
///
/// 🔴 `PageView` is described as mirroring `PageMarkView` "field for
/// field", and a comment saying so is not a check. The identical
/// claim in the raster was false: a `vec3<u32>` of padding made the
/// shader's struct twice the Rust one, and it surfaced as a
/// per-frame bind error rather than as a failing test.
#[test]
fn the_view_mirror_matches_the_shader() {
    let source = format!("{CLUSTER_COMMON}\n{PAGE_TABLE}\n{SOURCE}");
    let module = naga::front::wgsl::parse_str(&source).expect("the shader parses");
    let mut layouter = naga::proc::Layouter::default();
    layouter
        .update(module.to_ctx())
        .expect("the shader has a layout");
    let size = module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some("PageView"))
        .map(|(handle, _)| layouter[handle].size)
        .expect("`PageView` is declared");
    assert_eq!(size as usize, std::mem::size_of::<PageMarkView>());
}

/// Same size, wrong order — the failure a size check waves through.
///
/// 🔴 It has already happened here: `pool` went in after `paint` on
/// one side and before it on the other, and what broke was the page
/// DEBUG VIEW, which the change never touched.
#[test]
fn the_view_fields_line_up() {
    let mine = [
        (
            "world_from_clip",
            std::mem::offset_of!(PageMarkView, world_from_clip),
        ),
        (
            "eye_and_base",
            std::mem::offset_of!(PageMarkView, eye_and_base),
        ),
        ("sun", std::mem::offset_of!(PageMarkView, sun)),
        ("chain", std::mem::offset_of!(PageMarkView, chain)),
        ("strides", std::mem::offset_of!(PageMarkView, strides)),
        ("sampling", std::mem::offset_of!(PageMarkView, sampling)),
        ("pool", std::mem::offset_of!(PageMarkView, pool)),
        ("paint", std::mem::offset_of!(PageMarkView, paint)),
        ("life", std::mem::offset_of!(PageMarkView, life)),
        ("density", std::mem::offset_of!(PageMarkView, density)),
        ("halo", std::mem::offset_of!(PageMarkView, halo)),
    ];
    let source = format!("{CLUSTER_COMMON}\n{PAGE_TABLE}\n{SOURCE}");
    let module = naga::front::wgsl::parse_str(&source).expect("the shader parses");
    let theirs: Vec<(String, u32)> = module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some("PageView"))
        .and_then(|(_, ty)| match &ty.inner {
            naga::TypeInner::Struct { members, .. } => Some(
                members
                    .iter()
                    .map(|m| (m.name.clone().unwrap_or_default(), m.offset))
                    .collect(),
            ),
            _ => None,
        })
        .expect("`PageView` is a struct");
    assert_eq!(theirs.len(), mine.len(), "field count");
    for ((name, offset), (their_name, their_offset)) in mine.iter().zip(&theirs) {
        assert_eq!(name, their_name, "field order");
        assert_eq!(*offset as u32, *their_offset, "`{name}` starts elsewhere");
    }
}

/// A view's bits have to start on a word boundary, or emptying one
/// camera's region reaches into the other's.
#[test]
fn a_views_bits_start_on_a_word() {
    for levels in [1u32, 5, 17, 22] {
        let clipmap = ClipmapConfig { base: 1.28, levels };
        let stride = stride(PageConfig::default(), clipmap);
        assert_eq!(stride % 32, 0, "{levels} levels give a stride of {stride}");
        for slots in [1u32, 2, 102] {
            assert_eq!(span(PageConfig::default(), clipmap, slots) % 32, 0);
        }
    }
}
