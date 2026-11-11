# Changes from vello_svg 0.11.0

- `Cargo.toml`: `vello` 0.10 → the workspace's 0.11 (wgpu 30), `usvg` 0.48 → 0.49 (the one `resvg`
  already pulls), upstream's lint set and examples dropped.
- `src/render.rs`: maps usvg 0.49's new `BlendMode::PlusLighter` to `Compose::Plus`.
