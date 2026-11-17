//! The book's Rust examples, compiled as doctests so none names an API that is gone (#1725). Run
//! with a game's features: `cargo test --doc -j4 --features ui,physics,gravity,character,camera,audio,blockmesh,lighting,dynamic`.
//! The retired pages are left out: they describe code that no longer exists, on purpose.

macro_rules! pages {
    ($($name:ident => $path:literal),* $(,)?) => {
        $(
            #[doc = include_str!(concat!("../docs/book/src/", $path))]
            struct $name;
        )*
    };
}

pages! {
    ArchitectureComponentStorage => "architecture/component-storage.md",
    ArchitectureCrateGraph => "architecture/crate-graph.md",
    ArchitectureFramePacing => "architecture/frame-pacing.md",
    ArchitectureLighting => "architecture/lighting.md",
    ArchitectureProfiling => "architecture/profiling.md",
    ArchitectureRenderPipeline => "architecture/render-pipeline.md",
    ArchitectureResourceLifetime => "architecture/resource-lifetime.md",
    EditorDataAssets => "editor/data_assets.md",
    EditorOverview => "editor/overview.md",
    EditorPanels => "editor/panels.md",
    GuideAudio => "guide/audio.md",
    GuideBlocks => "guide/blocks.md",
    GuideCamera => "guide/camera.md",
    GuideCharacter => "guide/character.md",
    GuideColliders => "guide/colliders.md",
    GuideGettingStarted => "guide/getting-started.md",
    GuideGravity => "guide/gravity.md",
    GuideInput => "guide/input.md",
    GuideLighting => "guide/lighting.md",
    GuideMaterials => "guide/materials.md",
    GuidePhysics => "guide/physics.md",
    GuideScenes => "guide/scenes.md",
    GuideShaders => "guide/shaders.md",
    GuideShipping => "guide/shipping.md",
    GuideTime => "guide/time.md",
    GuideTextures => "guide/textures.md",
    GuideUiFiles => "guide/ui_files.md",
    GuideUi => "guide/ui.md",
    Introduction => "introduction.md",
    ReferenceDecisionsLog => "reference/decisions-log.md",
    ScriptingComponents => "scripting/components.md",
    ScriptingCreatingAProject => "scripting/creating-a-project.md",
    ScriptingFirstProject => "scripting/first-project.md",
    ScriptingSystems => "scripting/systems.md",
}
