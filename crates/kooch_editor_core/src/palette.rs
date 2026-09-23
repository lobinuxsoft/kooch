//! The editor's colour code, in one place.
//!
//! 🔴 Two rules, and everything here follows from them: **the icon says what a thing is, the colour
//! says which family it belongs to, and a STATE beats its family.** A row is scanned by shape at a
//! glance and read by colour when you stop on it, so a colour that means two things means nothing.

use egui::Color32;

/// What a thing is in — read when nothing is happening to it.
pub(crate) mod family {
    use super::Color32;

    /// Follows a prefab: the asset, and every entity of an instance.
    pub(crate) const PREFAB: Color32 = Color32::from_rgb(120, 180, 255);
    /// A scene file.
    pub(crate) const SCENE: Color32 = Color32::from_rgb(90, 200, 200);
    /// A mesh or a model.
    pub(crate) const MESH: Color32 = Color32::from_rgb(200, 164, 110);
    /// A material.
    pub(crate) const MATERIAL: Color32 = Color32::from_rgb(200, 155, 240);
    /// A shader, graph or WGSL.
    pub(crate) const SHADER: Color32 = Color32::from_rgb(240, 139, 192);
    /// A texture or any image.
    pub(crate) const TEXTURE: Color32 = Color32::from_rgb(224, 195, 90);
    /// A sound.
    pub(crate) const AUDIO: Color32 = Color32::from_rgb(154, 209, 106);
    /// An input map.
    pub(crate) const INPUT: Color32 = Color32::from_rgb(150, 235, 245);
    /// A block mesh.
    pub(crate) const BLOCK: Color32 = Color32::from_rgb(160, 190, 200);
}

/// What is happening to a thing — always wins over its family.
pub(crate) mod state {
    use super::Color32;

    /// The scene the game starts in.
    pub(crate) const MAIN_SCENE: Color32 = Color32::from_rgb(94, 207, 122);
    /// Edits that are not on disk.
    pub(crate) const DIRTY: Color32 = Color32::from_rgb(210, 150, 60);
}

/// The family colour for a typed asset, by the type the loader gives it. Only for a file whose
/// extension says nothing — every asset on disk has one, and the type is the fallback rather than
/// the rule.
pub(crate) fn of_type(type_name: &str) -> Option<Color32> {
    let colour = match type_name {
        "kooch_render::meshlet::asset::MeshletMesh" => family::MESH,
        "kooch_render::material::asset::Material" => family::MATERIAL,
        "kooch_input::actions::action::ActionMap" => family::INPUT,
        _ => return None,
    };
    Some(colour)
}

/// The family colour for a file, by extension.
///
/// 🔴 Read before the type: an asset the editor has a loader for arrives here already typed, and a
/// type this list does not name would otherwise lose the colour its extension knows — which is how
/// every prefab, block and texture came out plain.
pub(crate) fn of_extension(name: &str) -> Option<Color32> {
    let colour = match name.rsplit('.').next().unwrap_or("") {
        "scene" => family::SCENE,
        "prefab" => family::PREFAB,
        "material" => family::MATERIAL,
        "shader" | "wgsl" => family::SHADER,
        "png" | "jpg" | "jpeg" | "ktx2" | "dds" | "hdr" | "exr" | "tga" | "bmp" => family::TEXTURE,
        "wav" | "ogg" | "mp3" | "flac" => family::AUDIO,
        "inputaction" | "inputmap" => family::INPUT,
        "block" | "blockmesh" => family::BLOCK,
        "glb" | "gltf" | "obj" | "fbx" => family::MESH,
        // Project settings, code and notes keep the plain text colour on purpose: they are not
        // assets, and a colour for every file is a rainbow nobody reads.
        _ => return None,
    };
    Some(colour)
}

/// What a row is drawn in: its extension, then its type, then nothing.
pub(crate) fn of_asset(name: &str, type_name: Option<&str>) -> Option<Color32> {
    of_extension(name).or_else(|| type_name.and_then(of_type))
}

#[cfg(test)]
mod tests;
