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
//! # What it measured, 2026-08-20
//!
//! It was built to decompose the `11.11 ms` intercept of an older
//! device fit. **That floor no longer exists** — the whole shading pass
//! is 3.272 ms today — so the instrument answered a different question
//! than the one it was aimed at. Same session, same scene, same
//! upscaler, 60 s each:
//!
//! ```text
//! shade: compute (half rate)   baseline 3.272 ms   pad=252 48.152 ms
//! 44.88 ms / 252 sweeps  =  178 us per idle full-screen sweep
//! ```
//!
//! No control pass (`sgsr2`, `tonemap`, `shadows`, `blit`, `cluster
//! grid`) moved more than 0.16 ms against that 44.88 — 280:1 — and both
//! captures came back green on `read_capture --over-time`.
//!
//! The same measurement on a desktop 9070 XT reads **1.98 us**, a ratio
//! of 90x. It also decomposes there: at `KOOCH_SHADING_RATE=full` a
//! sweep costs 3.33 us rather than 4x1.98, so it is **1.53 us of fixed
//! dispatch cost plus 0.45 us of per-pixel work** at 320x180 — mostly
//! the command processor rather than the threads.
//!
//! 🎯 **What that makes it: 178 us per material in the PROJECT.**
//! `roll-a-ball` has three, so it pays 0.71 ms — 22 % of its own shading
//! pass, 7 % of its GPU frame. A game with twenty materials pays 3.7 ms,
//! which is 39 % of the GPU frame that currently meets the budget. An
//! unreferenced `.ron` in the materials folder costs 178 us a frame and
//! nothing says so.
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
//! hypothesis being false**. The unit tests below pin the arithmetic and
//! nothing can pin the wiring from inside the process.
//!
//! What pinned it was a desktop run before the device one: `gpu = 0.44
//! ms + 1.98 us per sweep` across pad 0 / 63 / 126 / 252, predicting
//! 0.565 / 0.690 / 0.939 against 0.57 / 0.70 / 0.94 measured. A straight
//! line through four points is a working instrument; one point is not.
//!
//! 🔴 **Use a pad in the hundreds, never single digits.** `pad=4` was
//! written into #885's protocol first and it is unmeasurable: four extra
//! sweeps are ~0.7 ms on the device, under the run-to-run drift this
//! roadmap documents at 16–37 %. 252 is the largest the 256-slot table
//! allows, and it is what produced 280:1. The cost of that choice is
//! that the frame stops fitting in a vblank and the pacing falls apart —
//! p99/median goes 1.84 to 2.67 — which is the load, not the knob.
//!
//! Image identity is not assumed either: the rendered pixels of both
//! shading paths were dumped at pad 0, 7 and 250 and compared by hash.
//! Identical, all six.
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
