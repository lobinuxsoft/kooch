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

/// What a thing is, for the two ways a row says it: its icon and its colour. One table, so the two
/// can never disagree — which is the whole point of a code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    Prefab,
    Scene,
    Mesh,
    Material,
    Shader,
    Texture,
    Audio,
    Input,
    Block,
    Settings,
    Code,
    Notes,
}

impl Family {
    /// The colour a row of this family is drawn in, or `None` for the ones that keep the plain text
    /// colour: settings, code and notes are not assets, and a colour for every file is a rainbow
    /// nobody reads.
    pub(crate) fn colour(self) -> Option<Color32> {
        let colour = match self {
            Self::Prefab => family::PREFAB,
            Self::Scene => family::SCENE,
            Self::Mesh => family::MESH,
            Self::Material => family::MATERIAL,
            Self::Shader => family::SHADER,
            Self::Texture => family::TEXTURE,
            Self::Audio => family::AUDIO,
            Self::Input => family::INPUT,
            Self::Block => family::BLOCK,
            Self::Settings | Self::Code | Self::Notes => return None,
        };
        Some(colour)
    }

    /// The icon that says the same thing the colour does.
    pub(crate) fn icon(self) -> &'static str {
        use crate::icons;

        match self {
            Self::Prefab => icons::PACKAGE,
            Self::Scene => icons::TREE_STRUCTURE,
            Self::Mesh => icons::CUBE,
            Self::Material => icons::SPHERE,
            Self::Shader => icons::SPARKLE,
            Self::Texture => icons::IMAGE,
            Self::Audio => icons::SPEAKER_HIGH,
            Self::Input => icons::GAME_CONTROLLER,
            Self::Block => icons::SHAPES,
            Self::Settings => icons::GEAR,
            Self::Code => icons::FILE_CODE,
            Self::Notes => icons::FILE_TEXT,
        }
    }
}

/// The family a file belongs to, by extension.
///
/// 🔴 Read before the type: an asset the editor has a loader for arrives already typed, and a type
/// the table below does not name would otherwise lose what its extension knows — which is how every
/// prefab, block and texture came out plain.
pub(crate) fn of_extension(name: &str) -> Option<Family> {
    let family = match name.rsplit('.').next().unwrap_or("") {
        "scene" => Family::Scene,
        "prefab" => Family::Prefab,
        "material" => Family::Material,
        "shader" | "wgsl" => Family::Shader,
        "png" | "jpg" | "jpeg" | "ktx2" | "dds" | "hdr" | "exr" | "tga" | "bmp" => Family::Texture,
        "wav" | "ogg" | "mp3" | "flac" => Family::Audio,
        "inputaction" | "inputmap" => Family::Input,
        "block" | "blockmesh" => Family::Block,
        "glb" | "gltf" | "obj" | "fbx" => Family::Mesh,
        "kooch" | "layers" | "rendersettings" | "buildpreset" | "toml" | "lock" | "ron" => {
            Family::Settings
        }
        "rs" => Family::Code,
        "md" | "txt" => Family::Notes,
        _ => return None,
    };
    Some(family)
}

/// The family of a typed asset, for a file whose extension says nothing.
pub(crate) fn of_type(type_name: &str) -> Option<Family> {
    let family = match type_name {
        "kooch_render::meshlet::asset::MeshletMesh" => Family::Mesh,
        "kooch_render::material::asset::Material" => Family::Material,
        "kooch_input::actions::action::ActionMap" => Family::Input,
        _ => return None,
    };
    Some(family)
}

/// What a row is: its extension, then its type, then nothing.
pub(crate) fn of_asset(name: &str, type_name: Option<&str>) -> Option<Family> {
    of_extension(name).or_else(|| type_name.and_then(of_type))
}

#[cfg(test)]
mod tests;
