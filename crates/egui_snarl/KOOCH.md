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

## Changes

- `Cargo.toml`: a standalone manifest, without the upstream workspace, the demo or `egui-probe`.
- `src/ui.rs`: the one inline test moved to `src/ui/tests.rs`, because the engine's vendoring ships no
  test code.
