//! The Image view (#1792): an image's 9-slice, kept in its `.meta` as Unity keeps a sprite's
//! borders, so every use of the image slices by it. `-kooch-slice` in a sheet stays an override.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::resource::Resources;
use kooch_render::texture::{Image, ImageImport};

/// The image open in the Image view.
#[derive(Clone)]
pub(crate) struct OpenImage {
    pub guid: Guid,
    pub path: PathBuf,
    /// RGBA8, row by row. Shared: the dock is handed a copy every frame.
    pub pixels: Arc<[u8]>,
    pub size: [u32; 2],
    /// Texels each edge keeps, top-right-bottom-left, as its `.meta` says.
    pub slice: [u16; 4],
    /// Set when the view should be brought to the front; taken once it has.
    pub focus_requested: bool,
}

/// Opens `path` in the Image view.
pub(crate) fn open(resources: &mut Resources, path: &Path) {
    let Some(guid) = resources
        .get::<AssetDatabase>()
        .and_then(|database| database.guid_for(path))
    else {
        tracing::error!(file = %path.display(), "not a known asset; cannot open it in the Image view");
        return;
    };
    let Some(handle) = crate::systems::asset_detail::load_handle::<Image>(guid, resources) else {
        tracing::error!(file = %path.display(), "failed to load the image for the Image view");
        return;
    };
    let Some(image) = resources
        .get::<kooch_core::assets::Assets<Image>>()
        .and_then(|images| images.get(handle))
    else {
        return;
    };
    let open = OpenImage {
        guid,
        path: path.to_path_buf(),
        pixels: image.data.as_slice().into(),
        size: [image.width, image.height],
        slice: import_of(path).slice,
        focus_requested: true,
    };
    resources.insert(open);
}

/// Whether `path` is a file the image loader reads, so the Image view can open it.
pub(crate) fn is_image(path: &Path) -> bool {
    let loader = kooch_render::texture::ImageLoader::srgb();
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            kooch_core::asset_loader::AssetLoader::extensions(&loader)
                .iter()
                .any(|known| ext.eq_ignore_ascii_case(known))
        })
}

/// The image's `[import]` table, or the defaults.
pub(crate) fn import_of(path: &Path) -> ImageImport {
    kooch_core::asset_meta::read_meta(path)
        .ok()
        .and_then(|meta| meta.import)
        .and_then(|table| kooch_core::toml::Value::Table(table).try_into().ok())
        .unwrap_or_default()
}

/// Sets the slice of `guid` as one step of its history.
pub(crate) fn set_slice(resources: &mut Resources, guid: Guid, slice: [u16; 4]) {
    crate::history::documents::record(
        resources,
        &crate::history::Document::Image(guid),
        "Set 9-slice",
        None,
    );
    write_slice(resources, guid, slice);
}

/// Writes `slice` into the `.meta` of `guid`, keeping the rest of its import settings. The bytes
/// do not change, so nothing re-uploads; the UI repacks by the `.meta`'s mtime.
pub(crate) fn write_slice(resources: &mut Resources, guid: Guid, slice: [u16; 4]) {
    let Some(path) = path_of(resources, guid) else {
        tracing::warn!(%guid, "9-slice not saved: no path in the AssetDatabase");
        return;
    };
    let written = kooch_core::asset_meta::read_meta(&path)
        .map_err(|error| error.to_string())
        .and_then(|mut meta| {
            let import = ImageImport {
                slice,
                ..import_of(&path)
            };
            meta.import =
                Some(kooch_core::toml::Table::try_from(import).map_err(|error| error.to_string())?);
            kooch_core::asset_meta::write_meta(&path, &meta).map_err(|error| error.to_string())
        });
    if let Err(error) = written {
        tracing::error!(path = %path.display(), %error, "failed to save the 9-slice");
        return;
    }
    if let Some(open) = resources
        .get_mut::<OpenImage>()
        .filter(|open| open.guid == guid)
    {
        open.slice = slice;
    }
}

pub(crate) fn path_of(resources: &Resources, guid: Guid) -> Option<PathBuf> {
    resources
        .get::<AssetDatabase>()
        .and_then(|database| database.entry(guid).map(|entry| entry.path.clone()))
}
