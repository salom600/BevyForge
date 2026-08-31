# BevyForge Troubleshooting

BevyForge ships as **one archive containing two programs** that must live side
by side:

| File | Role |
|------|------|
| `bevyforge` / `bevyforge.exe` | The editor UI (egui/OpenGL) |
| `bevyforge-runtime` / `bevyforge-runtime.exe` | The render engine (Bevy 0.19 / wgpu) that owns the scene, renders the viewport and runs Play mode |

The editor spawns the runtime automatically. If anything goes wrong the editor
tells you **exactly what happened** in a red banner at the top of the window, in
the status bar, and in the Console panel.

**Since 0.2.2 the editor is never a hollow shell**: even with the engine down,
all *editing* (hierarchy, creation, inspector, gizmos, undo, save/open) works on
an internal scene document and is synced to the engine automatically the moment
it connects. Only *Play mode, click-picking in the viewport and screenshots*
need the engine itself.

---

## 0. Quick self-diagnosis (do this first)

Run, from a terminal in the extracted folder:

```
bevyforge --doctor
```

It checks every link in the chain (paths → runtime binary → engine spawn → GPU
backends → IPC handshake), prints `PASS/FAIL` per item and writes
`bevyforge-doctor-report.txt` next to the executable. Send that file when asking
for help — it answers 90% of questions immediately.

Every launch also appends to `bevyforge.log` (next to the executable, or
`%LOCALAPPDATA%\BevyForge\bevyforge.log` when the folder is read-only).

---

## 1. “Editor opens but buttons do nothing”

Since 0.2.2 editing buttons **always work** (offline document mode). If they
appear dead on a version *before* 0.2.2, this is always the engine process
failing to start. The most common causes:

### a) **Fixed in 0.2.3 — Project menu dialogs never opened**
In 0.2.2 the *Project* menu items (New/Open Project, Open/Save Scene As) armed
a file dialog that was never rendered, so clicking them closed the menu with
no visible effect — they looked like fake buttons. 0.2.3 renders the dialog,
routes the picked folder/file (project switching even restarts the engine on
the new project root), and logs every step to the Console panel
(`dialog armed` → `picked: …` → `created '…'` / `active: …`). Verify your
version in the title bar: `BevyForge 0.2.3 · bevy 0.19`.

### b) The runtime executable is missing
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
