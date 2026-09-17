//! #1159 — a measured scope reaches the graph that wrote the shader, under the shader's name.

use std::path::{Path, PathBuf};

use kooch_core::Guid;

use super::*;
use crate::panels::inspector::AssetSource;

fn shader(path: &str) -> AssetCatalogEntry {
    AssetCatalogEntry {
        guid: Guid::new_v4(),
        path: PathBuf::from(path),
        display_name: Path::new(path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into(),
        source: AssetSource::Project,
        type_name: "Shader".to_owned(),
    }
}

/// The open graph holds an absolute path and the catalog a project-relative one: the cost has to
/// cross that, or the header reads "—" for every shader.
#[test]
fn an_open_graph_finds_its_cost() {
    let catalog = [
        shader("materials/rock.shader"),
        shader("materials/grass.shader"),
    ];
    let label = kooch_render::meshlet::shader_scope(Some(catalog[1].guid));
    let costs = vec![("shader built-in".to_owned(), 0.3), (label, 1.25)];

    let cost = shader_cost(
        &catalog,
        &costs,
        Path::new("/home/me/game/assets/materials/grass.shader"),
    );

    assert_eq!(cost, Some(1.25));
    assert_eq!(
        shader_cost(
            &catalog,
            &costs,
            Path::new("/home/me/game/assets/materials/rock.shader")
        ),
        None,
        "rock is not on screen and must not borrow another shader's time"
    );
}

#[test]
fn costs_read_by_name_descending() {
    let catalog = [
        shader("materials/rock.shader"),
        shader("materials/grass.shader"),
    ];
    let costs = vec![
        (
            kooch_render::meshlet::shader_scope(Some(catalog[0].guid)),
            0.4,
        ),
        ("shader built-in".to_owned(), 0.2),
        (
            kooch_render::meshlet::shader_scope(Some(catalog[1].guid)),
            1.1,
        ),
    ];

    let named = named_shader_costs(&catalog, &costs);

    let names: Vec<&str> = named.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["grass.shader", "rock.shader", "built-in"]);
}
