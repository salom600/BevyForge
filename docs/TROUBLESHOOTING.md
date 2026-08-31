# BevyForge Troubleshooting

BevyForge ships as **one archive containing two programs** that must live side
by side:

| File | Role |
|------|------|
| `bevyforge` / `bevyforge.exe` | The editor UI (egui/OpenGL) |
| `bevyforge-runtime` / `bevyforge-runtime.exe` | The render engine (Bevy 0.19 / wgpu) that owns the scene, renders the viewport and runs Play mode |

The editor spawns the runtime automatically. If anything goes wrong the editor
now tells you **exactly what happened** in a red banner at the top of the
window, in the status bar, and in the Console panel — it never silently shows
dead buttons.

---

## 1. “Editor opens but buttons do nothing”

This is always the engine process failing to start. Since 0.2.1 the editor
shows a red banner with the exact reason and retries automatically every few
seconds. The most common causes:

### a) The runtime executable is missing
You extracted or moved `bevyforge.exe` **alone** out of the archive, or your
zip tool skipped it.

**Fix:** extract the *whole* archive into one folder and run
`bevyforge.exe` from there. Both executables must sit in the same folder.

### b) Windows Defender / SmartScreen quarantined the runtime
Unsigned executables downloaded from the internet can be silently quarantined.

**Fix:** open *Windows Security → Protection history* and allow/restore
`bevyforge-runtime.exe`. On first launch, Windows SmartScreen shows
*More info → Run anyway*. The editor banner will say `not found next to the
editor` when this happens.

### c) Windows Defender controlled-folder access / antivirus blocking
**Fix:** add an exclusion for the folder you extracted BevyForge into.

### d) Missing DLLs (versions before 0.2.1)
0.2.1 statically links the C runtime — no VC++ redistributable is needed. For
older builds install the [Microsoft Visual C++ Redistributable
(x64)](https://aka.ms/vs/17/release/vc_redist.x64.exe).

---

## 2. “Engine crashes at startup” / GPU problems

The engine renders through **wgpu**, which supports Vulkan, DirectX 12,
Metal and OpenGL ES. BevyForge 0.2.1+ tries backends **in this order until
one works** (the successful one is shown in the status bar as `GPU: …`):

| Platform | Fallback chain |
|----------|----------------|
| Windows | auto (Vulkan→DX12) → DX12 → Vulkan → GL |
| Linux   | auto (Vulkan) → Vulkan → GL |
| macOS   | auto (Metal) → Metal |

### Manual override
Force a backend from the command line:

```
bevyforge-runtime --backend gl --project <project-dir>
```
or set the environment variable `FORGE_BACKEND=gl` (values: `vulkan`, `dx12`,
`gl`, `metal`, `all`). The equivalent wgpu variable `WGPU_BACKENDS` is also
honoured.

### No GPU at all (VM, remote desktop, very old card)
Use a software renderer:

- **Windows:** install [Mesa3D for Windows](https://github.com/pal1000/mesa-dist-win/releases)
  (`systemwidedeploy.cmd /deploy` or copy `x64/lvp_icd.x86_64.json` +
  `vk_swiftshader.dll` next to the runtime) — the lavapipe CPU driver gives
  full Vulkan on any PC. Then start normally; the status bar will show
  `GPU: lavapipe`.
- **Linux:** `sudo apt install mesa-vulkan-drivers` (lavapipe is included) —
  this is how BevyForge's own CI machines render.
- **macOS:** any Mac supported by Metal works.

### Update your drivers
Broken Vulkan drivers on old Intel iGPUs are the #1 cause of engine crashes.
Install the latest driver from your GPU vendor, or let the fallback chain pick
DX12/GL.

---

## 3. Port already in use

The engine binds TCP `127.0.0.1:48470` by default. If you run several editors:

```
bevyforge --port 48471
```

## 4. Where do I see engine logs?

Every stdout/stderr line of the engine, startup errors and backend attempts
appear in the editor's **Console panel → Output tab**, prefixed with
`stderr:` for crashes. Read the *last* lines — they contain the reason.

## 5. The editor window opens but is black / GL errors

The editor itself needs OpenGL 3.3+ (any driver from the last decade,
including Mesa llvmpipe). On exotic systems you can also run the editor under
Xvfb/headless CI exactly like BevyForge's test harness does.

## 6. Still stuck?

Open an issue at <https://github.com/salom600/BevyForge/issues> and paste the
red banner text + the last lines of the Console/Output tab.
