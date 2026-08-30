# BevyForge

A standalone, production-oriented graphical editor for the [Bevy](https://bevy.org) game
engine, built as a two-process system:

- **`bevyforge`** — the editor. An [egui](https://github.com/emilk/egui) desktop app with
  docked panels: scene hierarchy, asset browser, viewport, keyframe timeline, inspector,
  console, Rust compiler output, script editor and lighting controls.
- **`bevyforge-runtime`** — the engine. A headless [Bevy 0.19](https://crates.io/crates/bevy)
  process that owns the ECS `World`, renders offscreen and streams frames to the editor
  over a local socket.

```
┌─────────────────────────────┐        ┌──────────────────────────────┐
│ bevyforge (editor, egui)    │ frames │ bevyforge-runtime (Bevy)     │
│ hierarchy · inspector ·     │◀──────▶│ headless render · ECS world  │
│ timeline · console · assets │  IPC   │ play-mode systems · picking  │
└─────────────────────────────┘        └──────────────────────────────┘
```

## Features

| Panel | Capabilities |
|---|---|
| **Scene hierarchy** | tree view, search, click select, drag reparent, lock, visibility toggles, context-menu create/duplicate/delete |
| **Viewport** | live offscreen render stream (Scene tab = editor orbit camera, Game tab = scene camera), click-to-select via mesh ray casting, grid + selection outline, orbit/pan/zoom |
| **Inspector** | typed editing of transforms, materials, lights, cameras and gameplay scripts; add/remove components; rename; active/visibility toggle |
| **Timeline** | per-entity translation/rotation/scale keyframes, scrubbing, looped playback, drag keyframes, capture-from-selection |
| **Assets** | live filesystem browser for `project/assets`, scene double-click open, image previews, script double-click edit |
| **Console** | engine log stream with level filter |
| **Rust Compiler** | `cargo check` runner with JSON diagnostics, error/warning counters, click-to-open-line |
| **Script editor** | edit the `forge_scripts` gameplay components with syntax highlighting, save + check loop |
| **Environment** | ambient light, sun orbit/illuminance/shadows, tonemapping, EV100 exposure, fog, clear colour, grid/outline toggles |
| **Play Mode** | snapshot → run gameplay systems → full state restore on stop |

Every control is wired to a real effect — there are no decorative widgets.

## Building

```sh
cargo build --release
# editor + runtime binaries:
target/release/bevyforge
target/release/bevyforge-runtime
```

Linux system requirements for the *runtime* renderer: a Vulkan or GL driver (wgpu).
The editor uses OpenGL via glow. No audio/windowing system dependencies are required
by the runtime (it renders headless).

## Running

```sh
target/release/bevyforge                 # creates/opens ~/BevyForgeProjects/Demo
target/release/bevyforge --project path/to/project
target/release/bevyforge --connect 48470 # attach to a running runtime
bevyforge-runtime --screenshot shot.png --width 1920 --height 1080
```

## Architecture

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full document, including the
Bevy 0.19 onboarding study that drove the design (scene system rework, `bevy_light`
extraction, headless render pipeline, message-based events).

Crate map:

| Crate | Role |
|---|---|
| `forge_ipc` | wire protocol (postcard over TCP), framed transport |
| `forge_editor_core` | project model, undo/redo, cargo diagnostics parser, process supervisor |
| `forge_scripts` | user-editable gameplay components + play-mode systems |
| `forge_runtime` | the Bevy engine process |
| `bevyforge` | the editor UI |

## CI

`.github/workflows/build.yml` builds Linux (ubuntu-24.04) and Windows (windows-2022)
release binaries, runs the unit tests, and uploads runnable archives as artifacts on
every push to `main`.

## License

Dual-licensed under MIT or Apache-2.0, matching Bevy's licensing.
