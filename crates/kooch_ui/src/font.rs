//! A project's `.ttf`/`.otf` (#1749): an asset, so it is catalogued and ships with the game, and
//! registered with [`UiFonts`] so `font-family` finds it by its family name.

use std::sync::Arc;

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::asset_loader::{AssetLoader, AssetResult, AssetServer, LoadContext};
use kooch_core::resource::Resources;
use parley::fontique::Blob;

use crate::UiFonts;
use crate::documents::{path_of, read_bytes};

/// What the asset database types a font file as.
pub const FONT_TYPE: &str = "kooch_ui::font::UiFont";

/// The families the engine embeds, and so always ships, by the GUID of their file under the
/// engine's `assets/fonts/`: the editor picks them like any font asset.
pub const ENGINE_FONTS: [(Guid, &str); 2] = [
    (
        Guid::from_bytes([
            143, 133, 151, 34, 117, 205, 74, 127, 183, 60, 32, 96, 53, 13, 159, 181,
        ]),
        "Inter",
    ),
    (
        Guid::from_bytes([
            136, 161, 241, 127, 78, 237, 74, 194, 146, 230, 12, 5, 145, 204, 132, 26,
        ]),
        "Fira Code",
    ),
];

/// A font file's bytes.
#[derive(Debug, Clone)]
pub struct UiFont {
    pub bytes: Arc<[u8]>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UiFontLoader;

impl AssetLoader<UiFont> for UiFontLoader {
    fn extensions(&self) -> &[&'static str] {
        &["ttf", "otf"]
    }

    fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<UiFont> {
        Ok(UiFont {
            bytes: bytes.into(),
        })
    }
}

kooch_core::register_asset!(UiFont, UiFontLoader);

impl UiFonts {
    /// Every family `font-family` can name, the engine's first, each with the file holding it.
    pub fn families(&self) -> &[(Guid, Box<str>)] {
        &self.families
    }
}

/// Registers every project font not registered yet. Rescans only when the database's size
/// changes: a font replaced in place keeps the family it had.
pub fn load_fonts(resources: &mut Resources) {
    let Some(count) = resources.get::<AssetDatabase>().map(AssetDatabase::len) else {
        return;
    };
    if resources
        .get::<UiFonts>()
        .is_none_or(|fonts| fonts.scanned == count)
    {
        return;
    }
    let mut server = resources.remove::<AssetServer>();
    let mut read = Vec::new();
    if let (Some(database), Some(fonts)) =
        (resources.get::<AssetDatabase>(), resources.get::<UiFonts>())
    {
        for (guid, _) in database.entries_of_type(FONT_TYPE) {
            if fonts.families.iter().any(|(own, _)| *own == guid) {
                continue;
            }
            match path_of(database, guid).and_then(|path| read_bytes(server.as_mut(), &path)) {
                Ok(bytes) => read.push((guid, bytes)),
                Err(error) => tracing::warn!("UI font not loaded: {error}"),
            }
        }
    }
    if let Some(server) = server {
        resources.insert(server);
    }
    let Some(fonts) = resources.get_mut::<UiFonts>() else {
        return;
    };
    fonts.scanned = count;
    for (guid, bytes) in read {
        let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(bytes);
        for (family, _) in fonts.font.collection.register_fonts(Blob::new(data), None) {
            if let Some(name) = fonts.font.collection.family_name(family) {
                let name: Box<str> = name.into();
                if !fonts.families.iter().any(|(_, own)| *own == name) {
                    fonts.families.push((guid, name));
                }
            }
        }
    }
    fonts.families.sort_by(|a, b| a.1.cmp(&b.1));
}
