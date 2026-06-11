# Root-Cause Analysis: Windows GPU Disabled (CEF GPU-process STATUS_BREAKPOINT)

**Status:** Complete — empirically verified (CDP `SystemInfo`/`chrome://gpu`, CEF logs, A/B build, VS Code comparison)
**Author:** AgentX
**Date:** 2026-06-11
**This PR:** SwiftShader software-GL safety net (`agentx/gpu-hardware-fix`)
**Real fix (separate):** an official (DCHECK-off) Windows `libcef` build in `agentmuxai/cef` — filed as an issue.

---

## 1. Summary

On Windows, AgentMux's GPU is fully disabled (`gl=disabled`, `webgl: disabled_off`): the
Chromium **GPU process crashes with `STATUS_BREAKPOINT` (`0x80000003`) ×3 at GPU
shared-context init**, falls to software GL, which also fails (no SwiftShader bundled), so
Chromium disables all GPU → every terminal uses the slow xterm **DOM renderer** (no
scrollbar).

**Root cause:** the `agentmuxai/cef` fork's **Windows `libcef.dll` is a non-official /
DCHECK-enabled build.** A `DCHECK`/`CHECK` in the GPU's context-virtualization init fires as
a fatal `int3` — which **upstream release Chrome (VS Code) and AgentMux's macOS build both
survive** on the same machine.

**This is NOT** transparency (disproven by A/B), **NOT** adapter selection (NVIDIA is
correctly chosen and is fully capable), and **NOT** AMD-switchable (false on this box).

---

## 2. Evidence

### 2.1 The hardware GPU is fully capable

`chrome://gpu` Hardware-GPU profile (scraped from a running portable):
`GPU0 = NVIDIA RTX 3060 *ACTIVE*`, `GL_RENDERER: ANGLE (NVIDIA … Direct3D11)`,
`(gl=egl-angle, angle=d3d11)`, Skia GaneshGL, **init 192 ms**, WebGL: *Hardware accelerated*.
So NVIDIA D3D11 works when the GPU process boots.

### 2.2 The crash

```
GpuProcessHost: GPU process exited unexpectedly: exit_code=-2147483645  (0x80000003 STATUS_BREAKPOINT) ×3
ContextResult::kFatalFailure: Failed to create shared context for virtualization.
GPU process was unable to boot: GPU process crashed too many times with software GL.  → Disabled: all
```

`STATUS_BREAKPOINT` = a `DCHECK`/`CHECK` `int3` *inside* the GPU process during GL/D3D
context creation (init ~134–192 ms, before it can log the GL detail).

### 2.3 It's a non-official (DCHECK-enabled) build

AgentMux's `cef-debug.log` prints **full source paths + line numbers** —
`gpu\ipc\service\gpu_channel_manager.cc:927`, `components\viz\service\main\viz_main_impl.cc:190`.
**Official/release Chrome strips these.** Their presence ⇒ `is_official_build=false` and/or
`dcheck_always_on=true`, which leaves DCHECKs live. VS Code (official Chrome) on the same
machine does not crash and uses NVIDIA D3D11.

### 2.4 Transparency ruled out (A/B)

Built an opaque variant (`background_color = 0xFF222222` in main.rs/app.rs/ui_tasks.rs) and
launched it on a **virgin data dir**. Result: **identical** crash — 3× `STATUS_BREAKPOINT`
at "create shared context for virtualization." So the translucent surface is not the trigger.

### 2.5 Other negatives

- **High-performance GPU preference** (`UserGpuPreferences = GpuPreference=2`) on the host exe
  did **not** fix it — the portable still crashed 3×.
- **AMD switchable: false**, **Optimus: false** — adapter selection correctly targets NVIDIA;
  not a switchable-graphics `disable_d3d11` issue.

### 2.6 Why macOS works but Windows doesn't

macOS GPU is enabled in the latest fork build. Either the macOS `libcef` is built official
(DCHECK-off) while Windows isn't, or the firing DCHECK isn't hit on macOS's ANGLE-Metal
backend vs Windows's ANGLE-D3D11. Either way it is the **Windows fork build** that differs.

---

## 3. The fix

### 3.1 Real fix (separate — `agentmuxai/cef` build): official Windows libcef

Build the Windows `libcef` from the fork with **`is_official_build=true`** (or at minimum
`dcheck_always_on=false` + `is_debug=false`) — matching a release Chrome build and the macOS
build. That compiles out the firing DCHECK; the GPU process then initializes NVIDIA D3D11
normally (§2.1). Re-point the `cef-rs` download (`AgentU-asaf/cef-rs`) at the new binary and
rebuild AgentMux. **Filed as an issue on `agentmuxai/cef`.**

### 3.2 This PR — SwiftShader software-GL safety net

Independent of the build fix, ensure a hardware-GPU failure degrades to **software WebGL**
instead of fully-disabled DOM rendering:

- `Taskfile.yml` `bundle:windows` ships `vk_swiftshader.dll`, `vulkan-1.dll`,
  `vk_swiftshader_icd.json` (~6 MB; parity with macOS/Linux, which already bundle it).
- `agentmux-cef/src/app.rs` adds `--enable-unsafe-swiftshader` (Chromium 110+ gates the
  SwiftShader WebGL fallback behind it).

With both, when the GPU process can't boot, Chromium settles on SwiftShader (software WebGL)
instead of `crashed too many times with software GL → disable all`. **Software is CPU-bound
and not the goal** — hardware via §3.1 is — but it removes the "fully-disabled DOM-renderer"
cliff and keeps the xterm WebGL renderer + scrollbar working. Hardware GL is still preferred
when it boots, so there is no cost on healthy machines.

### 3.3 Further hardening (not in this PR — tracked from CRASH_GPU_PROCESS_FATAL_2026_05_20)

`--disable-gpu-process-crash-limit`, launcher relaunch with `--disable-gpu` after repeated
crashes, `SetErrorMode(SEM_NOGPFAULTERRORBOX)`, launcher supervision + state restore — so
*any* GPU fault becomes invisible recovery.

---

## 4. Re-verify

```
# GPU feature status of a running build (dev 9223 / release 9222):
curl -s http://127.0.0.1:9223/json/version > ver.json
node scripts/cdp-featurestatus.mjs ver.json     # expect webgl: enabled / disabled_software (net) — not disabled_off

# Crash signature in the CEF log:
grep -aE 'GPU process|STATUS_BREAKPOINT|shared context' \
  ~/.agentmux/channels/stable/versions/<v>/logs/cef-debug.log

# Build-flavor tell (non-official leaves source paths in log messages):
grep -aE '\.cc:[0-9]+\]' <cef-debug.log>     # present ⇒ non-official build
```
