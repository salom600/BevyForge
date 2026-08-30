# BevyForge Architecture & Bevy 0.19.1 Onboarding Study

> Phase 0 deliverable — engine onboarding as required by the project specification.
> Everything below was verified against the actual `v0.19.1` source tree.

## 1. Bevy 0.19.1 — what changed and what we build on

BevyForge targets **bevy 0.19.1** (latest stable, crates.io, Aug 2026). The 0.17 → 0.19
window contained several structural changes that directly shaped this project:

| Area | 0.16-era (prior knowledge) | 0.19.1 (verified in source) | BevyForge response |
|---|---|---|---|
| Scenes | `DynamicScene` + `bevy_scene` RON round-trip | **Removed.** Replaced by BSN ("Bevy Scene Notation") — code-first `bsn!` macro, `ResolvedScene`, template patching (`crates/bevy_scene/src/lib.rs`) | BevyForge stores its **own typed scene format** (`ForgeScene`, serde RON) — see §3 |
| Lights | `bevy_pbr::{PointLight, DirectionalLight, SpotLight}`, `AmbientLight` resource | New **`bevy_light`** crate; `AmbientLight` is now a component with `#[require(Camera)]`, global default is the `GlobalAmbientLight` resource (brightness in cd/m², default 80) | Lighting panel edits `GlobalAmbientLight` + per-light components (`crates/bevy_light/src/ambient_light.rs`) |
| Shadows | `shadows_enabled` | `shadow_maps_enabled` (+ new `contact_shadows_enabled`) | Protocol field mapping updated |
| Tonemapping | `AcesFilmic` | Renamed `AcesFitted`; variants: None, Reinhard, ReinhardLuminance, AcesFitted, AgX, SomewhatBoringDisplayTransform, TonyMcMapface, BlenderFilmic, KhronosPbrNeutral (`bevy_core_pipeline/src/tonemapping/mod.rs`) | `TonemappingKind` enum mirrors the available set |
| Events | `EventWriter` / `app.add_event` | **`MessageWriter` / `app.add_message`** (events renamed to messages) | Runtime uses `MessageWriter<AppExit>` |
| Render target | `Camera { target: RenderTarget }` | `RenderTarget` is its **own component** inserted alongside `Camera3d`; `Image::new_target_texture(w, h, format, None)` helper | Offscreen viewport camera built exactly like `examples/app/headless_renderer.rs` |
| Headless | `MinimalPlugins` hackery | Official pattern: `DefaultPlugins.set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit })` **without** `bevy_winit` feature + `ScheduleRunnerPlugin::run_loop(...)` | Runtime is feature-gated headless (see §2) |
| GPU readback | `gpu_readback` module | `ImageCopier` pattern: buffer copy node in the `RenderGraph` schedule + `receive_image_from_buffer` after `RenderSystems::Render`, `RenderDevice::poll(PollType::wait_indefinitely())` | Frame streaming implemented per the official headless_renderer example |
| Fog | `FogSettings` resource | `DistanceFog` **component** with `FogFalloff::Linear { start, end }` (`bevy_pbr/src/fog.rs`) | Lighting tab exposes linear fog |
| Exposure | `Exposure::indirect(f32)` | `Exposure { ev100: f32 }` component (EV100 units) | Editor slider works in EV100 |
| Camera fov | `Projection` enum component | unchanged (`bevy_camera/src/projection.rs`, `fov: f32` under `Projection::Perspective`) | Inspector edits `Projection` |
| Picking | `bevy_picking` mesh backend w/ pointer events | `ray_mesh_intersection(ray, transform, positions, normals, indices, uvs, Backfaces)` public helper (`bevy_picking/src/mesh_picking/ray_cast/intersections.rs`) | Runtime does **manual ray casting** driven by IPC `Pick` commands (no pointer input exists headless) |
| Gizmos | `gizmos.grid_3d(...)` | `grid_3d(isometry: impl Into<Isometry3d>, cell_count: UVec3, spacing: Vec3, color)` (`bevy_gizmos/src/grid.rs`) | Editor grid overlay |
| ECS | stable | `Name(HashedStr)` in `bevy_ecs::name`; `ChildOf`/`Children` relationships; exclusive systems for world surgery | Hierarchy + commands |

### Core architecture takeaways (from the source study)

1. **ECS-first**: everything is data (`Component`/`Resource`) plus scheduled `System`s.
   BevyForge adds *no* new engine paradigms — the editor is just another set of systems
   reading/writing the same `World`.
2. **Plugins are the unit of composition**: the runtime composes `DefaultPlugins` (feature-
   pruned) + `ForgeRuntimePlugin`; the editor scripts crate is registered via
   `ForgeScriptsPlugin::build` calling `register_type::<T>()` for every gameplay component.
3. **Schedules over loops**: headless mode swaps the winit runner for
   `ScheduleRunnerPlugin::run_loop(1/60 s)`. All editor IPC handling is a normal `Update`
   system — exclusive (`&mut World`) so it can perform structural ECS changes.
4. **Render world is a separate world**: GPU readback must hop through `ExtractSchedule` →
   render-graph node → `Render`-scheduled system with a channel back to the main world.
   BevyForge's streaming viewport follows the official `headless_renderer` example exactly.

## 2. Process architecture

```
┌──────────────────────────────┐          127.0.0.1 TCP (postcard framing)
│  bevyforge (editor, egui)    │◀──────────────────────────────────────────────
│  ┌─────────┐ ┌────────────┐  │   RuntimeToEditor: frames, hierarchy,       │
│  │ Panels  │ │ Undo stack │  │   components, logs, stats, anim, notices    │
│  └────┬────┘ └─────┬──────┘  │                                             │
│       ▼            ▼          │   EditorToRuntime: spawn/delete/reparent,   │
│  ┌──────────────────────┐    │   field edits, scene io, play/stop, pick,   │
│  │ forge_editor_core    │    │   keyframes, environment, screenshots       │
│  └──────────┬───────────┘    │                                             │
└─────────────┼────────────────┘                                             │
              │ spawns child, parses FORGE_PORT handshake                    │
┌─────────────▼────────────────┐                                             │
│  bevyforge-runtime (bevy)    │◀────────────────────────────────────────────│
│  ┌───────────────────────┐   │                                             │
│  │ ForgeRuntimePlugin    │   │  * headless DefaultPlugins (no winit/audio) │
│  │  - ipc_server system  │   │  * ScheduleRunnerPlugin @ 60 Hz             │
│  │    (exclusive)        │   │  * offscreen Camera3d → Rgba8UnormSrgb      │
│  │  - scene io (typed    │   │  * ImageCopier → RGB8 → IPC frames @30 Hz   │
│  │    ForgeScene RON)    │   │  * play-mode systems (forge_scripts)        │
│  │  - picking, grid      │   │  * custom tracing layer → IPC logs          │
│  └───────────────────────┘   │                                             │
└──────────────────────────────┘
```

Why two processes (user-selected architecture):
- **Stability**: an engine crash (shader compile panic, GPU driver fault) cannot take the
  editor down; the editor shows a friendly "runtime exited" state and can respawn it.
- **Decoupling**: the UI never blocks the engine loop; heavy cargo builds and file IO run
  editor-side while the ECS keeps ticking.
- **Honesty**: no simulated state — every pixel and every hierarchy row comes from the
  actual Bevy `World`.

## 3. Data ownership & the typed scene format

The editor keeps **no authoritative ECS state**. It mirrors:
- `Vec<HierNode>` — rebuilt from runtime hierarchy pushes,
- `ComponentData` for the selected entity (typed rows),
- `AnimState` + tracks, `EnvironmentSettings`, `Stats`, `SceneInfo`.

`DynamicScene` no longer exists in 0.19, and BSN is code-first. Rather than fight that,
BevyForge defines **`ForgeScene`** — a fully typed serde RON document mirroring the
`forge_ipc` data model (entities with transform/visibility/mesh-primitive/material/light/
camera/script components + parent links + animation tracks + environment). Primitives are
re-created deterministically on load, so there are no asset-handle serialisation problems.
Round-trip tests live in `forge_runtime::scene_io`.

Play mode uses the same format as its snapshot: on `SetPlayMode{playing:true}` the runtime
serialises the current world to an in-memory `ForgeScene`, enables the `forge_scripts`
behaviour systems; on stop it despawns user entities and re-spawns from the snapshot.

## 4. Crate map

| Crate | Kind | Purpose |
|---|---|---|
| `forge_ipc` | lib | Protocol enums, transport (length-prefixed postcard over TCP), unit tests |
| `forge_editor_core` | lib | Project model (`BevyForge.toml`), undo/redo batches, cargo JSON diagnostics parser, runtime process supervisor |
| `forge_scripts` | lib | User-editable gameplay components (`Player`, `CharacterController`, `Health`, `Inventory`, `Rotator`, `Orbiter`, `LinearMover`, `PingPongMover`) + play-mode systems |
| `forge_runtime` | bin | The engine process: headless Bevy, IPC server, scene IO, picking, frame streaming, CLI screenshot mode |
| `bevyforge` | bin | The editor: eframe/egui panels mirroring the 500.png design, theme, docking-lite layout, script editor, compiler panel |

## 5. Build & CI

- Workspace `[profile.release]`: `lto = "thin"`, `codegen-units = 16`.
- Dev profile: `opt-level = 1`, `debug = 0` (sandbox-friendly compile budget).
- Runtime bevy features (headless, no sudo-needed system deps):
  `default-features = false, features = ["std","bevy_asset","bevy_pbr","bevy_gizmos","bevy_log","png","zstd"]`
  — no `bevy_winit` (no X11/alsa/udev build deps), renders offscreen through wgpu.
- GitHub Actions: `build.yml` — two matrix jobs (ubuntu-24.04, windows-2022), cargo
  release build, artefact zips (`BevyForge-<ver>-linux-x64.zip`, `...-windows-x64.zip`)
  containing both binaries + `project/` scaffold + README.
- Local visual verification on this sandbox: xvfb + Mesa llvmpipe (extracted locally to
  `~/mesa-local`, `LD_LIBRARY_PATH` + `__EGL_VENDOR_LIBRARY_FILENAMES`), runtime on
  software GL via `WGPU_BACKEND=gl`.

## 6. Editor UX mapping (design file 500.png → implementation)

| Design element | Implementation |
|---|---|
| Menu bar + play/pause/stop | egui `TopBottomPanel`; File/GameObject/Component/Window/Help menus; play controls gated by runtime connection |
| Scene panel (Hierarchy/Entities tabs) | tree from `HierNode` list; click select, drag reparent, context menu, search filter |
| Assets panel | real filesystem browser of `project/assets`; image thumbnails; double-click scene opens; double-click .rs opens script editor |
| Viewport tab strip + toolbar | streaming RGB8 texture; Perspective/Ortho buttons, gizmo tool state, grid/outline toggles; click-to-select via Pick IPC |
| Timeline/Animation | per-entity Translation/Rotation/Scale tracks; add/remove/move keyframes; transport (play/pause/scrub/loop); runtime interpolation system |
| Inspector | typed rows per `ComponentData` (Vec3 XYZ fields with axis colours, drag-edit floats, colour swatches, booleans); `+ Add Component` menu; remove component |
| Console + Output | batched `LogEntry` stream, level filter, severity colouring, clear |
| Rust Compiler panel | runs `cargo check --message-format=json -p forge_scripts`, streams parsed diagnostics, error/warning counters, click-to-open line |
| Status bar | runtime-reported FPS, frame ms, entity count, system count, memory, wgpu backend; connection state dot |

Every visible control is wired to a real effect; nothing is decorative.

## 7. Transform gizmos (drag manipulation)

The manipulator overlay is painted **entirely in the editor process** as a screen-space
egui layer on top of the streamed frame; no 3D geometry is involved in hit-testing.

```
runtime                                editor (egui)
───────                                ──────────────
CameraInfo { view_proj, eye }  ──────▶ project world → screen (Mat4 inverse, unproject)
BeginGizmoGesture  ◀──────────────    drag start on a handle (hit-test in px space)
MoveEntity / RotateEntityWorld /       pointer ray ∩ axis line / constraint plane
ScaleEntityBy   ◀──────────────        → relative commands at frame rate
EndGizmoGesture   ◀──────────────
GestureDone { pre, post }      ──────▶ exact undo pair pushed (runtime-side glam euler)
```

Design decisions:

- **Relative commands** (`MoveEntity`, `RotateEntityWorld`, `ScaleEntityBy`) are applied
  to the live `Transform`, so a drag never drifts from inspector refresh latency; the
  runtime is the single source of truth mid-gesture.
- **Undo pairs are computed runtime-side** (`BeginGizmoGesture` snapshots, `GestureDone`
  reports pre/post with glam `EulerRot::XYZ` degrees) so the editor never reimplements
  quaternion → euler conversion.
- **Handles**: per-axis arrows + XY/XZ/YZ plane squares + camera-plane centre dot
  (translate); three projected rings, edge-on ring hidden (rotate); axis squares +
  uniform centre (scale). Screen-constant size via world-units-per-pixel derived from
  the view ray at the anchor.
- **Snapping**: Ctrl holds 0.25 m translation grid, 15° rotation steps, 0.05 scale steps;
  snapping quantises the *absolute* accumulated value so no error accumulates across
  drag frames.
- Locked entities render the gizmo dimmed and ignore gestures; `EditorLocked` is also
  enforced runtime-side.

## 8. Icon system

`bevyforge/src/icons.rs` paints ~48 glyphs (translate, rotate, scale, transport, files,
objects, log levels, UI chrome) with egui path primitives in a 16-unit design space —
no icon font, no raster assets; icons inherit the theme and stay crisp at any DPI.
