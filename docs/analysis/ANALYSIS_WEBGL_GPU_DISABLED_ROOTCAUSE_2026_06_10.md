# Root-Cause Analysis: WebGL Unavailable in AgentMux (GPU process disabled)

**Status:** Complete — evidence-backed via CDP + CEF logs + system inventory
**Author:** AgentX
**Date:** 2026-06-10
**Fix:** `agentx/gpu-swiftshader-fallback-and-status` (this PR)

---

## 1. Summary

WebGL is unavailable in AgentMux on this machine because **Chromium's GPU process
crashes at initialization and, after 3 crashes, Chromium disables the GPU entirely**
(`gl=disabled, angle=none`). xterm then falls back to the DOM renderer, which in xterm v6
has no native scrollbar (scroll is wheel-only).

Two independent causes combine:

1. **Environmental trigger:** a **Parsec Virtual Display Adapter** is installed alongside
   the real GPUs. Chromium's GPU process cannot bring up a hardware D3D11/GL device in that
   display context and aborts with `STATUS_BREAKPOINT` (`0x80000003`) at ~130 ms init.
2. **AgentMux contributing factor:** the Windows bundle step **strips SwiftShader**
   (`vk_swiftshader.dll`, `vulkan-1.dll`), so there is **no software-GL fallback**. Chromium
   goes straight from "hardware GPU crashed" to "GPU fully off." (Linux/macOS bundles keep
   SwiftShader — see Taskfile.yml.)

It is **not** dev-specific, **not** a feature branch, and **not** a CEF version mismatch: the
downloaded **portable v0.44.0** build shows the identical disabled state, and `libcef.dll`
is correctly `148.0.9 / chromium-148.0.7778.180` (matches the host).

---

## 2. Evidence

### 2.1 GPU is off (both dev :9223 and portable :9222)

`SystemInfo.getInfo` → `featureStatus` on **both** running builds:

```
opengl: disabled_off    webgl: disabled_off    webgpu: disabled_off
gpu_compositing: disabled_software    2d_canvas: disabled_software
glImplementationParts: "(gl=disabled,angle=none)"   glRenderer/Vendor/Version: "Disabled"
processCrashCount: 3    driverBugWorkarounds: []
```

Page-side: `canvas.getContext('webgl'|'webgl2'|'experimental-webgl')` all return null with
`"...GL_VENDOR = Disabled, GL_RENDERER = Disabled... ErrorMessage = BindToCurrentSequence failed"`.

### 2.2 The GPU process crash sequence (dev CEF debug log)

`~/.agentmux/dev/<branch>/<hash>/logs/cef-debug.log`:

```
123048.292  ERROR gpu_process_host.cc:999  GPU process exited unexpectedly: exit_code=-2147483645
123048.292  WARN  gpu_process_host.cc:1441 The GPU process has crashed 1 time(s)
123048.468  ...                              exit_code=-2147483645  → crashed 2 time(s)
123048.646  ...                              exit_code=-2147483645  → crashed 3 time(s)
123048.726  ERROR gpu_channel_manager.cc:927 ContextResult::kFatalFailure:
                                             Failed to create shared context for virtualization.
```

- `exit_code=-2147483645` = `0x80000003` = **STATUS_BREAKPOINT** — a Chromium CHECK/DCHECK or
  `__debugbreak()`, i.e. the process *loaded fine* then asserted during GL/D3D init (init
  times 132/133/0 ms). A missing DLL would be `0xC0000135` (DLL_NOT_FOUND), which this is not.
- Same exit code recurs historically across versions in the launcher log (v0.38, v0.39,
  v0.42) → a persistent, environment-level condition, not a one-off.

### 2.3 The machine has a virtual display adapter

`Win32_VideoController`:

| Adapter | Driver | Note |
|---|---|---|
| **Parsec Virtual Display Adapter** | 0.45.0.0 (2024) | virtual display (remote streaming) |
| AMD Radeon(TM) Graphics | 32.0.21043.5001 (2026) | iGPU |
| NVIDIA GeForce RTX 3060 | 32.0.15.9186 (2026) | dGPU |

`SessionName=Console`, `SessionId=1`. Parsec's virtual adapter is the well-known trigger for
Chromium/Electron GPU-process crashes: when the GPU process targets the virtual display's
adapter, hardware D3D11 device creation fails and (in this CEF build) trips a CHECK →
`STATUS_BREAKPOINT`. Repeated 3× → Chromium's "GPU process keeps crashing" policy disables
the GPU for the session.

### 2.4 No software fallback is bundled (the AgentMux factor)

Dev runtime (`dist/cef-dev/runtime`) **and** the downloaded portable runtime both contain
`libcef.dll`, `libEGL.dll`, `libGLESv2.dll`, `d3dcompiler_47.dll` — but **not**
`vk_swiftshader.dll` / `vulkan-1.dll`.

`Taskfile.yml` bundle step (Windows branch) explicitly stripped them; the Linux/macOS
branches deliberately keep SwiftShader "so GPU processes start on machines without a system
Vulkan loader." So on Windows there was nothing for Chromium to fall back to once the
hardware GPU process died → `webgl: disabled_off` (not `disabled_software`).

---

## 3. Causal chain

```
Parsec Virtual Display Adapter is the GPU/display context
        ↓
Chromium GPU process can't create a hardware D3D11/GL device → CHECK fail (STATUS_BREAKPOINT)
        ↓
GPU process crashes 3× in ~350 ms → Chromium disables GPU for the session (gl=disabled)
        ↓
No SwiftShader bundled on Windows → no software-GL fallback → webgl: disabled_off
        ↓
xterm WebglAddon can't load → DOM renderer
        ↓
xterm v6 DOM renderer has no native viewport scrollbar → wheel-only scroll
```

---

## 4. Fix (this PR)

### 4.1 Bundle SwiftShader on Windows + enable it

- `Taskfile.yml` `bundle:windows` now copies `vk_swiftshader.dll`, `vulkan-1.dll`,
  `vk_swiftshader_icd.json` into `dist/cef/` (parity with macOS/Linux).
- `agentmux-cef/src/app.rs` appends `--enable-unsafe-swiftshader`. Chromium 110+ gates the
  SwiftShader-for-WebGL fallback behind this switch; without it, even with the DLLs present,
  `getContext('webgl')` returns null once hardware GPU is off. Hardware GL is still preferred
  when available — this is a fallback only, no cost on healthy machines.

Result: a hardware-GPU failure degrades to **software WebGL** instead of disabling WebGL
outright — the xterm WebGL renderer (and its native scrollbar) keep working on
virtual-display / headless / broken-driver machines, exactly the remote-agent environments
AgentMux runs in.

### 4.2 Make the degraded state visible (this PR)

A status-bar GPU indicator (`frontend/util/gpuutil.ts`, `frontend/app/statusbar/GpuStatus.tsx`)
shows `GPU HW/SW/off` in the bottom-left, and the BackendStatus popover (click the uptime)
gains a GPU section: WebGL class, unmasked renderer/vendor, and the terminal's actual renderer
(WebGL vs DOM, via `termRendererAtom` set in `termwrap.ts`). So the fallback is observable
rather than silent.

### 4.3 Environmental note (no code)

The hardware GPU process should also initialize when the session is driven by a real display
adapter (physical monitor on the console) rather than the Parsec virtual display; disabling
the Parsec Virtual Display Adapter in Device Manager typically lets Chromium's GPU process
start. 4.1 is what makes AgentMux *resilient* to it regardless.

### 4.4 Not a fix

- Bumping drivers — both real GPUs already have 2026 drivers; the trigger is the virtual
  adapter / display context.
- The launcher's `--disable-gpu` ladder — host-crash-level; it did **not** fire (host cmdline
  was just `--url=...`); the GPU *process* crashes are Chromium-internal.

---

## 5. Impact

Until this fix, **every terminal on an affected machine uses the xterm DOM renderer**: slower
for large/fast output, and **no visible scrollbar** (wheel-only). The companion scrollbar-gap
fix stands on its own; restoring WebGL via §4.1 (or a visible DOM-renderer scrollbar) is the
complete story.

---

## 6. Appendix: how to re-verify

```
# GPU feature status of any running build (dev 9223 / portable 9222):
curl -s http://127.0.0.1:9223/json/version > ver.json
node scripts/cdp-featurestatus.mjs ver.json

# GPU crash sequence:
grep -aE 'GPU process|ContextResult|shared context' \
  ~/.agentmux/dev/<branch>/<hash>/logs/cef-debug.log

# Adapters:
pwsh -NoProfile -Command "Get-CimInstance Win32_VideoController | Format-List Name,DriverVersion"
```
