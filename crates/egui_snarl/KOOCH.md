# egui-snarl, in-tree

A fork of [egui-snarl](https://github.com/zakarumych/egui-snarl) 0.11.0 by Zakarum, licensed MIT OR
Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`). The first commit adding this directory is the published
crate unchanged. Every later change is listed here.

## Why a fork

The Shader Graph panel (#1211) needs three things the published widget does not offer:

- a plain click on a node that selects it
- a way to set the selection from outside, for select-all and for selecting pasted nodes
- the drawn size of each node, so a group can fit its members

None of these can be added from outside the crate: the selection and the node sizes live in private
types.

## Updating

```sh
python3 .github/scripts/update_egui_snarl.py 0.12.0 --dry-run   # what would happen
python3 .github/scripts/update_egui_snarl.py 0.12.0             # do it
```

The script merges every file three ways: upstream at the version this fork is based on, this fork,
and upstream at the new version. The changes below carry over on their own. A conflict appears only
where upstream changed the same lines, and it is left marked in the file. The script also prints
upstream's dependency tables when they moved. 0.12.0 moves egui to 0.36, so it goes with the
workspace's egui bump.

A trial run onto 0.12.0 (2026-09-18) merged with no conflicts.

## Changes

- `Cargo.toml`: a standalone manifest, without the upstream workspace, the demo or `egui-probe`.
- `src/ui.rs`: the one inline test moved to `src/ui/tests.rs`, because the engine's vendoring ships no
  test code.
- `src/ui.rs`: a plain primary click on a node selects it alone, and starting to drag an unselected
  node selects it. Upstream selects with Shift only. A plain click on the background clears the
  selection; upstream needs Ctrl.
- `src/ui.rs`, `src/ui/state.rs`: `set_selected_nodes(id, ctx, nodes)` replaces the selection, read
  by the widget's next frame.
- `src/ui.rs`, `src/ui/state.rs`: `get_node_rects(id, ctx)` returns each node's rect from the last
  frame, in graph space. It is kept every frame, not only when a box selection ends.
- `src/ui/viewer.rs`: the default header's title is not a selectable label. A selectable label
  takes the click that selects the node.

