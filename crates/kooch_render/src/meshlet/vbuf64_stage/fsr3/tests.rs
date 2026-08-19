use super::*;

/// A transliteration has to survive naga before it can be judged by
/// eye, and a WGSL error at pipeline-creation time is a panic inside a
/// GPU test that says nothing about which line is wrong.
fn validates(label: &str, pass: &str) {
    let wgsl = source(pass);
    let module = naga::front::wgsl::parse_str(&wgsl)
        .unwrap_or_else(|e| panic!("{label} should parse: {}", e.emit_to_string(&wgsl)));
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .unwrap_or_else(|e| panic!("{label} should validate: {e:?}"));
}

#[test]
fn the_prepare_inputs_shader_validates() {
    validates("fsr3 prepare_inputs", PREPARE_INPUTS_SOURCE);
}

#[test]
fn the_reduce_shader_validates() {
    validates("fsr3 reduce", REDUCE_SOURCE);
}

#[test]
fn the_reactivity_shader_validates() {
    validates("fsr3 prepare_reactivity", REACTIVITY_SOURCE);
}

#[test]
fn the_instability_shader_validates() {
    validates("fsr3 luma_instability", INSTABILITY_SOURCE);
}

/// The largest of the five by a wide margin, and the only one that runs
/// at output resolution.
#[test]
fn the_accumulate_shader_validates() {
    validates("fsr3 accumulate", ACCUMULATE_SOURCE);
}

/// 🔴 The uniform block is declared twice — once in WGSL and once as a
/// `#[repr(C)]` here — and nothing but this test connects them. A field
/// added to one and not the other reads garbage on the GPU without any
/// error anywhere.
///
/// 96 is not arbitrary: the uniform address space requires the size to
/// be a multiple of 16, and every `vec2<f32>` in the block is 8-aligned.
#[test]
fn the_uniform_block_is_96_bytes() {
    assert_eq!(std::mem::size_of::<Fsr3Ubo>(), 96);
}

/// Each field of the WGSL struct, in order, against the Rust one. A
/// reordering is the failure this catches: the sizes still match, the
/// pipeline still builds, and every constant lands in the wrong place.
#[test]
fn the_two_uniform_declarations_agree() {
    let wgsl = COMMON_SOURCE;
    let start = wgsl.find("struct Fsr3Params {").expect("the block exists");
    let end = start + wgsl[start..].find("\n}").expect("the block closes");
    let fields: Vec<&str> = wgsl[start..end]
        .lines()
        .filter_map(|line| {
            line.trim()
                .split(':')
                .next()
                .filter(|_| line.contains(": "))
        })
        .filter(|name| !name.starts_with("//"))
        .collect();

    assert_eq!(
        fields,
        [
            "render_size",
            "output_size",
            "render_size_rcp",
            "output_size_rcp",
            "jitter",
            "prev_jitter",
            "downscale",
            "near",
            "exposure",
            "reset",
            "frame_index",
            "delta_pre_exposure",
            "jitter_sequence_length",
            "debug",
            "_pad0",
            "_pad1",
            "_pad2",
        ],
    );
}

/// The dispatch grid must cover every pixel. An odd size that rounds
/// down leaves a strip of the image never written — which reads as a
/// black band at the right edge and is exactly the class of bug
/// `render_scale` already produced once on an odd panel width.
#[test]
fn the_grid_covers_an_odd_size() {
    let (x, y) = groups((1281, 721));
    assert!(x * GROUP >= 1281, "{x} groups cover {} px", x * GROUP);
    assert!(y * GROUP >= 721, "{y} groups cover {} px", y * GROUP);
}

/// Zero would be a dispatch of nothing, which wgpu accepts and which
/// leaves the output at whatever it held.
#[test]
fn the_grid_is_never_empty() {
    assert_eq!(groups((0, 0)), (1, 1));
}
