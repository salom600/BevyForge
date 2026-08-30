# BevyForge Project — Demo

This directory is a BevyForge **project**: open it with

```sh
bevyforge --project /path/to/this/project
```

## Layout
- `assets/scenes/*.scn.ron` — scenes (typed BevyForge RON format)
- `assets/scripts/` — notes for gameplay scripts (the compiled gameplay crate
  lives in `crates/forge_scripts` of the workspace; the in-editor Script Editor
  and Rust Compiler panel work on that crate)
- `assets/prefabs`, `assets/materials`, `assets/textures`, `assets/meshes` — content folders

The bundled `main.scn.ron` mirrors the BevyForge design blueprint: ground,
platforms, props group (crate/barrel/terminal), a rigged Player prefab and an
animated Spinning Cube (Rotator component). Press **Play** in the editor to see
the gameplay systems run, **Stop** to restore the exact pre-play state.
