//! `KOOCH_SHADING_PAD` — the instrument for the 11 ms floor (#885).
//!
//! Both shading paths issue one full-screen dispatch (or one
//! depth-tested full-screen draw) **per registered material slot**, and
//! `MaterialPipeline::shading_slots` counts every `Material` the
//! `AssetDatabase` knows about rather than the ones the frame draws. A
//! tile that owns none of a slot's pixels still reads the R64 vbuf,
//! chases `visible_meshlets` and `instances` off that read, and waits on
//! three unconditional barriers.
//!
//! The device fit that motivates this is `shade = 11.11 ms + 1.06 ms per
//! light`. #821, #824, #820 and #826 all attacked the per-light term;
//! the 11.11 ms intercept is 37 % of the pass and nobody had broken it
//! down. These sweeps are the first candidate, because they are
//! per-pixel by construction and scale with something the game author
//! changes without knowing.
//!
//! # Why a knob rather than editing the project's materials
//!
//! Removing materials from `roll-a-ball` measures the same term with
//! four other things moving: the pack, the `AssetDatabase`, the texture
//! pool, and the picture itself. Padding the slot range moves **one**
//! thing. A padded slot's `material_id` matches no instance, so `mine`
//! is never set and every store in `material_pbr_compute.wgsl` is inside
//! that branch — the frame is bit-identical and the only difference is
//! the sweeps.
//!
//! 🔴 The padding is **appended**, never prepended: the fragment path
//! clears the colour target on its first pass and loads on the rest
//! (`two_pass.rs`, `color_load`), so a pad slot at index 0 would clear
//! the frame and every real material would be composited over nothing.
//!
//! # 🔴 The run needs a positive control, and no test can be it
//!
//! `compute_shading_parity.rs` says it in the comment above
//! `the_light_limit_darkens_both_paths`: *a knob that silently does
//! nothing produces a capture that looks like an answer*. This one is
//! worse than `KOOCH_LIGHT_LIMIT` in exactly that respect — the cap
//! darkens the picture, so a test can see it work, while a pad that
//! never reached the dispatch loop would look **identical to the
//! hypothesis being false**, which is the conclusion it would then
//! license. The unit tests below pin the arithmetic and nothing can pin
//! the wiring from inside the process.
//!
//! What pins it is the measurement itself: **`KOOCH_SHADING_PAD=200`
//! first**. Two hundred full-screen sweeps cannot be free on any
//! hardware, so a run that does not collapse is a broken instrument and
//! not a refuted candidate. Only after that does a small pad mean
//! anything.
//!
//! The knob lives in the environment because the editor is not where
//! this can be measured — a frame on the OneXFly is, launched through
//! Steam, which is how `KOOCH_CLUSTERING`, `KOOCH_SPECULAR_FLOOR`,
//! `KOOCH_SHADING_RATE` and `KOOCH_LIGHT_LIMIT` all got their readings.

use std::ops::Range;

/// Appends `KOOCH_SHADING_PAD` sweeps that own no pixel, clamped so the
/// range never leaves the slot table `max` describes.
///
/// Clamped rather than asserted: this is a measurement knob read from a
/// launch option, and a typo must cost a wrong number rather than a
/// crash on a handheld nobody is sitting in front of.
pub(crate) fn padded_slots(slots: Range<u32>, max: u32) -> Range<u32> {
    extend_slots(slots, pad_from_environment(), max)
}

/// The arithmetic, apart from the read, so a test can exercise the
/// clamp and the append without touching the process environment.
fn extend_slots(slots: Range<u32>, pad: u32, max: u32) -> Range<u32> {
    slots.start..slots.end.saturating_add(pad).min(max)
}

/// `KOOCH_SHADING_PAD=<n>`, read once.
fn pad_from_environment() -> u32 {
    static PAD: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *PAD.get_or_init(|| {
        let pad = parse_pad(std::env::var("KOOCH_SHADING_PAD").ok().as_deref());
        if pad != 0 {
            tracing::info!(
                target: "kooch_render::vbuf64_stage",
                "KOOCH_SHADING_PAD={pad}: {pad} extra full-screen shading sweeps that \
                 own no pixel. The picture does not change — this measures what an \
                 idle sweep costs, which is the first candidate for the ~11 ms floor \
                 the light fit leaves behind (#885)",
            );
        }
        pad
    })
}

/// The parse, apart from the read, so a test can exercise it without
/// touching the process environment.
///
/// Anything unparseable is zero, the same as unset: a typo during a
/// measurement run must not silently change what is being measured.
fn parse_pad(raw: Option<&str>) -> u32 {
    raw.map(str::trim)
        .filter(|raw| !raw.is_empty())
        .map(|raw| raw.parse::<u32>().unwrap_or(0))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
