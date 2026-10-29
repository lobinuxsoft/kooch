//! Materials for an imported glTF (#1656): one `.material` per glTF material, wired to the
//! textures the file brings, and the first one recorded on the mesh's `.meta` so a drop spawns it
//! ready to see.

use std::any::type_name;
use std::path::{Path, PathBuf};

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::asset_meta;
use kooch_core::resource::Resources;
use kooch_render::material::{MATERIAL_EXTENSION, Material};
use kooch_render::texture::Image;

/// The `[import]` key on a mesh's `.meta` naming the material a spawn assigns.
const MATERIAL_KEY: &str = "material";

/// Writes the materials of `mesh` and the textures a `.glb` embeds; returns every file created.
pub(super) fn import(mesh: &Path) -> Vec<PathBuf> {
    let mut created = Vec::new();
    let Ok(bytes) = std::fs::read(mesh) else {
        return created;
    };
    let Ok(gltf) = gltf::Gltf::from_slice(&bytes) else {
        return created;
    };
    let dir = mesh.parent().unwrap_or(Path::new(""));
    let stem = mesh
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("mesh")
        .to_owned();

    let images: Vec<Option<Guid>> = gltf
        .images()
        .map(|image| image_guid(&gltf, &image, dir, &stem, &mut created))
        .collect();
    let texture = |info: Option<gltf::Texture>| info.and_then(|t| images[t.source().index()]);

    let mut first = None;
    for (index, source) in gltf.materials().enumerate() {
        let label = source.name().map(file_safe).unwrap_or(index.to_string());
        let file = dir.join(format!("{stem}_{label}.{MATERIAL_EXTENSION}"));
        if !file.exists() {
            let pbr = source.pbr_metallic_roughness();
            let emissive = source.emissive_factor();
            let material = Material {
                base_color: pbr.base_color_factor(),
                metallic: pbr.metallic_factor(),
                roughness: pbr.roughness_factor(),
                emissive: emissive[0].max(emissive[1]).max(emissive[2]),
                albedo: texture(pbr.base_color_texture().map(|t| t.texture())),
                metal_roughness: texture(pbr.metallic_roughness_texture().map(|t| t.texture())),
                normal: texture(source.normal_texture().map(|t| t.texture())),
                ..Material::default()
            };
            let Ok(text) = ron::ser::to_string_pretty(&material, Default::default()) else {
                continue;
            };
            if let Err(e) = std::fs::write(&file, text) {
                tracing::error!(file = %file.display(), error = %e, "import: material not written");
                continue;
            }
            created.push(file.clone());
        }
        let guid = typed_guid(&file, type_name::<Material>());
        if first.is_none() {
            first = guid;
        }
    }

    if let Some(guid) = first {
        record_material(mesh, guid);
    }
    created
}

/// The material a spawn of the mesh behind `guid` should wear, as the import recorded it.
pub(crate) fn imported_material(resources: &Resources, mesh: Guid) -> Option<Guid> {
    let path = resources.get::<AssetDatabase>()?.entry(mesh)?.path.clone();
    asset_meta::read_meta(&path)
        .ok()?
        .import?
        .get(MATERIAL_KEY)?
        .as_str()?
        .parse()
        .ok()
}

/// The guid of an image the document names: a file beside it, or bytes it embeds, written out.
fn image_guid(
    gltf: &gltf::Gltf,
    image: &gltf::Image,
    dir: &Path,
    stem: &str,
    created: &mut Vec<PathBuf>,
) -> Option<Guid> {
    let file = match image.source() {
        gltf::image::Source::Uri { uri, .. } if uri.starts_with("data:") => return None,
        gltf::image::Source::Uri { uri, .. } => dir.join(uri),
        gltf::image::Source::View { view, mime_type } => {
            let ext = match mime_type {
                "image/jpeg" => "jpg",
                _ => "png",
            };
            let label = image
                .name()
                .map(file_safe)
                .unwrap_or(image.index().to_string());
            let file = dir.join(format!("{stem}_{label}.{ext}"));
            if !file.exists() {
                let buffer = match view.buffer().source() {
                    gltf::buffer::Source::Bin => gltf.blob.clone()?,
                    gltf::buffer::Source::Uri(uri) => std::fs::read(dir.join(uri)).ok()?,
                };
                let bytes = buffer.get(view.offset()..view.offset() + view.length())?;
                std::fs::write(&file, bytes).ok()?;
                created.push(file.clone());
            }
            file
        }
    };
    file.is_file()
        .then(|| typed_guid(&file, type_name::<Image>()))
        .flatten()
}

fn typed_guid(file: &Path, asset_type: &str) -> Option<Guid> {
    asset_meta::read_or_create_typed(file, asset_type)
        .ok()
        .map(|meta| meta.guid)
}

fn record_material(mesh: &Path, guid: Guid) {
    let Ok(mut meta) = asset_meta::read_or_create(mesh) else {
        return;
    };
    meta.import
        .get_or_insert_with(Default::default)
        .insert(MATERIAL_KEY.to_owned(), guid.to_string().into());
    if let Err(e) = asset_meta::write_meta(mesh, &meta) {
        tracing::warn!(mesh = %mesh.display(), error = %e, "import: material not recorded on the mesh");
    }
}

fn file_safe(name: &str) -> String {
    name.chars()
        .map(|c| match c.is_alphanumeric() || c == '-' || c == '_' {
            true => c,
            false => '_',
        })
        .collect()
}
