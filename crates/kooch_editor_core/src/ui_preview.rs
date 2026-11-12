//! The project's UI in the Game View: built here from its files while stopped, so it can be
//! designed without running the game (#1697), and received from the host in Play (#1691). Each
//! has its own atlas: the two processes pack their glyphs apart.

use kooch_core::gpu::GpuContext;
use kooch_core::resource::Resources;
use kooch_render::ui::UiPass;

use crate::remote_session::RemoteState;

/// The host's last UI frame, and the pass that draws it.
#[derive(Default)]
pub(crate) struct RemoteUi {
    pub list: kooch_ecs::UiDrawList,
    pass: Option<UiPass>,
}

fn remote_playing(resources: &Resources) -> bool {
    resources
        .get::<RemoteState>()
        .is_some_and(|state| state.session.is_some() && state.playing)
}

pub(crate) fn start_preview_system(resources: &mut Resources) {
    resources.insert(kooch_ui::UiTrees::default());
    resources.insert(kooch_ui::UiFonts::default());
    resources.insert(kooch_ui::GlyphAtlas::default());
    resources.insert(kooch_ui::UiShaders::default());
}

/// The preview's atlas generation while Play has it released.
#[derive(Default)]
struct Released(Option<u32>);

/// 🔴 In Play the host's atlas is drawn, so the preview's (64 MB with a HUD image) is freed until
/// Stop, then rebuilt from the files (#1891).
fn release_preview(resources: &mut Resources) {
    if resources
        .get::<Released>()
        .is_some_and(|released| released.0.is_some())
    {
        return;
    }
    let generation = resources
        .remove::<kooch_ui::GlyphAtlas>()
        .map_or(0, |atlas| atlas.generation());
    resources.remove::<UiPass>();
    resources.remove::<kooch_ecs::UiDrawList>();
    resources.insert(Released(Some(generation)));
}

fn restore_preview(resources: &mut Resources) {
    let Some(generation) = resources
        .get_mut::<Released>()
        .and_then(|released| released.0.take())
    else {
        return;
    };
    resources.insert(kooch_ui::GlyphAtlas::after(generation));
}

/// Stopped, the documents are the editor's to build and lay out; in Play the host's frame stands.
pub(crate) fn preview_ui_system(resources: &mut Resources) {
    if remote_playing(resources) {
        release_preview(resources);
        return;
    }
    restore_preview(resources);
    kooch_ui::prune_trees(resources);
    kooch_ui::load_fonts(resources);
    kooch_ui::build_documents(resources);
    kooch_ui::load_images(resources);
    kooch_ui::load_shaders(resources);
    kooch_ui::layout_documents(resources);
    kooch_ui::draw_documents(resources);
}

/// Over the Game View, after its post-process, as in the game.
pub(crate) fn draw(
    gpu: &GpuContext,
    encoder: &mut wgpu::CommandEncoder,
    resources: &mut Resources,
    target: &wgpu::TextureView,
    size: (u32, u32),
) {
    if !remote_playing(resources) {
        if let Some(pass) = resources
            .get_mut::<RemoteUi>()
            .and_then(|ui| ui.pass.as_mut())
        {
            pass.park_videos();
        }
        kooch_render::ui::draw_frame(gpu, encoder, resources, target, size);
        return;
    }
    let Some(mut ui) = resources.remove::<RemoteUi>() else {
        return;
    };
    let pass = ui
        .pass
        .get_or_insert_with(|| UiPass::new(gpu.device(), gpu.format()));
    pass.draw(
        gpu.device(),
        gpu.queue(),
        encoder,
        target,
        size,
        &mut ui.list,
        false,
    );
    resources.insert(ui);
}
