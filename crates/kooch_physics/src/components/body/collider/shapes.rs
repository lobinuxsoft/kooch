//! The `shape` discriminant, labels and per-shape fields. Hiding is display only. Scenes store the
//! number, so shapes only append — renumbering turns authored capsules into something else.

use kooch_ecs::reflect::{FieldChoice, FieldCondition};

/// Ball of radius `radius`.
pub const SHAPE_SPHERE: u32 = 0;
/// Box of half-extents `half_extents`.
pub const SHAPE_CUBOID: u32 = 1;
/// Capsule along local Y: `radius` plus `half_height` excluding caps.
pub const SHAPE_CAPSULE: u32 = 2;
/// Cylinder along local Y, flat caps.
pub const SHAPE_CYLINDER: u32 = 3;
/// Cylinder with a rim fillet of `border_radius`.
pub const SHAPE_ROUND_CYLINDER: u32 = 4;
/// Cone along local Y, apex up.
pub const SHAPE_CONE: u32 = 5;
/// Infinite plane through the shape's centre, solid below `normal`.
pub const SHAPE_HALF_SPACE: u32 = 6;
/// Line from `point_a` to `point_b`.
pub const SHAPE_SEGMENT: u32 = 7;
/// Triangle `point_a`, `point_b`, `point_c`.
pub const SHAPE_TRIANGLE: u32 = 8;
/// Convex hull of the source mesh's vertices.
pub const SHAPE_CONVEX_HULL: u32 = 9;
/// The source mesh, decomposed into convex parts.
pub const SHAPE_CONVEX_DECOMPOSITION: u32 = 10;
/// The source mesh's triangles, as they are.
pub const SHAPE_TRIMESH: u32 = 11;
/// The source mesh's vertices, joined as a line strip.
pub const SHAPE_POLYLINE: u32 = 12;
/// Grid cells the source mesh's vertices land in.
pub const SHAPE_VOXELS: u32 = 13;
/// The source mesh, voxelised at build time.
pub const SHAPE_VOXELIZED_MESH: u32 = 14;
/// Triangles the entity generated itself, as blocks use; `mesh` is not read.
pub const SHAPE_OWN_MESH: u32 = 15;

/// Dropdown labels, fewer than shapes: `Segment`, `Triangle`, `Polyline`, `Voxels` and
/// `VoxelizedMesh` still load and work but answer no author question. Words from Unity and Unreal,
/// with the trade in the label.
pub static SHAPE_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "Sphere",
        value: SHAPE_SPHERE as i64,
    },
    FieldChoice {
        label: "Cuboid",
        value: SHAPE_CUBOID as i64,
    },
    FieldChoice {
        label: "Capsule",
        value: SHAPE_CAPSULE as i64,
    },
    FieldChoice {
        label: "Cylinder",
        value: SHAPE_CYLINDER as i64,
    },
    FieldChoice {
        label: "Cylinder (rounded rim)",
        value: SHAPE_ROUND_CYLINDER as i64,
    },
    FieldChoice {
        label: "Cone",
        value: SHAPE_CONE as i64,
    },
    FieldChoice {
        label: "Half-space (infinite plane)",
        value: SHAPE_HALF_SPACE as i64,
    },
    FieldChoice {
        label: "Convex — one hull",
        value: SHAPE_CONVEX_HULL as i64,
    },
    FieldChoice {
        label: "Convex — several pieces",
        value: SHAPE_CONVEX_DECOMPOSITION as i64,
    },
    FieldChoice {
        label: "Complex — exact, static only",
        value: SHAPE_TRIMESH as i64,
    },
];

/// Shapes built from a mesh asset — wider than [`SHAPE_CHOICES`], so unlabelled ones still resolve.
/// One list for the Inspector and [`ShapeSpec`](super::ShapeSpec).
pub const MESH_DERIVED: &[u32] = &[
    SHAPE_OWN_MESH,
    SHAPE_CONVEX_HULL,
    SHAPE_CONVEX_DECOMPOSITION,
    SHAPE_TRIMESH,
    SHAPE_POLYLINE,
    SHAPE_VOXELS,
    SHAPE_VOXELIZED_MESH,
];

/// Which shapes read `radius`: everything round about local Y.
pub static RADIUS_WHEN: FieldCondition = FieldCondition {
    field: "shape",
    values: &[
        SHAPE_SPHERE as i64,
        SHAPE_CAPSULE as i64,
        SHAPE_CYLINDER as i64,
        SHAPE_ROUND_CYLINDER as i64,
        SHAPE_CONE as i64,
    ],
};

/// Which shapes read `half_extents`: only the box.
pub static HALF_EXTENTS_WHEN: FieldCondition = FieldCondition {
    field: "shape",
    values: &[SHAPE_CUBOID as i64],
};

/// Which shapes read `half_height`: the ones with a length along Y.
pub static HALF_HEIGHT_WHEN: FieldCondition = FieldCondition {
    field: "shape",
    values: &[
        SHAPE_CAPSULE as i64,
        SHAPE_CYLINDER as i64,
        SHAPE_ROUND_CYLINDER as i64,
        SHAPE_CONE as i64,
    ],
};

/// Which shapes read `border_radius`: only the filleted cylinder.
pub static BORDER_RADIUS_WHEN: FieldCondition = FieldCondition {
    field: "shape",
    values: &[SHAPE_ROUND_CYLINDER as i64],
};

/// Which shapes read `normal`: only the half-space.
pub static NORMAL_WHEN: FieldCondition = FieldCondition {
    field: "shape",
    values: &[SHAPE_HALF_SPACE as i64],
};

/// Shapes that read `point_a`/`point_b`; unlisted now, kept editable for existing scenes.
pub static ENDPOINTS_WHEN: FieldCondition = FieldCondition {
    field: "shape",
    values: &[SHAPE_SEGMENT as i64, SHAPE_TRIANGLE as i64],
};

/// Which shapes read `point_c`: only the triangle.
pub static POINT_C_WHEN: FieldCondition = FieldCondition {
    field: "shape",
    values: &[SHAPE_TRIANGLE as i64],
};

/// Which shapes read `mesh`.
pub static MESH_WHEN: FieldCondition = FieldCondition {
    field: "shape",
    values: &[
        SHAPE_CONVEX_HULL as i64,
        SHAPE_CONVEX_DECOMPOSITION as i64,
        SHAPE_TRIMESH as i64,
        SHAPE_POLYLINE as i64,
        SHAPE_VOXELS as i64,
        SHAPE_VOXELIZED_MESH as i64,
    ],
};

/// Which shapes read `voxel_size`.
pub static VOXEL_SIZE_WHEN: FieldCondition = FieldCondition {
    field: "shape",
    values: &[SHAPE_VOXELS as i64, SHAPE_VOXELIZED_MESH as i64],
};

/// Which shapes read `voxel_solid`: only the one that fills a volume.
pub static VOXEL_SOLID_WHEN: FieldCondition = FieldCondition {
    field: "shape",
    values: &[SHAPE_VOXELIZED_MESH as i64],
};

/// Whether this discriminant needs geometry from outside physics.
pub fn is_mesh_derived(shape: u32) -> bool {
    MESH_DERIVED.contains(&shape)
}

/// Whether a discriminant's geometry is the entity's own. 🔴 `mesh` is not consulted, or a `.block`
/// reaches the glTF parser.
pub fn is_own_mesh(shape: u32) -> bool {
    shape == SHAPE_OWN_MESH
}
