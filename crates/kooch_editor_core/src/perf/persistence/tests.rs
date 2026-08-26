use crate::perf::{HudVisibility, PinnedSections};

/// The user's spec, verbatim: a fresh layout shows the frame-time
/// card and nothing else stacked.
#[test]
fn the_default_overlay_is_frame_time() {
    let hud = HudVisibility::default();
    assert!(hud.frame_time_card);
    assert!(!hud.info_card);
    assert!(!hud.shadow_pages_window);
    let p = hud.pinned;
    for (name, on) in [
        ("debug", p.debug),
        ("frame", p.frame),
        ("project", p.project),
        ("system", p.system),
        ("render", p.render),
        ("meshlet", p.meshlet),
        ("cpu_frame", p.cpu_frame),
        ("remote", p.remote),
    ] {
        assert!(!on, "section {name} is stacked by default");
    }
}

/// The toggles round-trip; the per-frame fields do not — they are
/// what would have turned every frame into a disk write.
#[test]
fn the_choices_survive_the_trip() {
    let hud = HudVisibility {
        info_card: true,
        shadow_pages_window: true,
        pinned: PinnedSections {
            meshlet: true,
            ..Default::default()
        },
        panel_visible: true,
        ..Default::default()
    };
    let text = ron::ser::to_string(&hud).expect("serializes");
    assert!(
        !text.contains("panel_visible") && !text.contains("system_section"),
        "a transient field reached the file: {text}"
    );
    let back: HudVisibility = ron::from_str(&text).expect("parses");
    assert!(back.info_card && back.shadow_pages_window && back.pinned.meshlet);
    assert!(back.frame_time_card, "a default field was lost");
    assert!(!back.panel_visible, "the per-frame flag was persisted");
    assert!(back.system_section, "the skipped field lost its default");
}
