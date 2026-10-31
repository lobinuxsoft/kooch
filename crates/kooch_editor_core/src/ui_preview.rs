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
}

/// Stopped, the documents are the editor's to build and lay out; in Play the host's frame stands.
pub(crate) fn preview_ui_system(resources: &mut Resources) {
    if remote_playing(resources) {
        return;
    }
    kooch_ui::prune_trees(resources);
    kooch_ui::build_documents(resources);
    kooch_ui::load_images(resources);
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
    );
    resources.insert(ui);
}
