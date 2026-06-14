# SPEC: Linux GPU Backend Precedence (capability-probed ANGLE selection)

**Date:** 2026-06-13
**Status:** Implemented in PR #1394 — replaces the initial VMware DMI gate
**Author:** AgentU
**Related:** `agentmux-cef/src/app.rs` (`on_before_command_line_processing`, `is_vmware_guest`), `agentmux-cef/Cargo.toml`, `frontend/app/view/term/termwrap.ts`, `frontend/util/gpuutil.ts`, `frontend/app/statusbar/GpuStatus.tsx`

---

## 1. Problem

On Linux, CEF 148 defaults ANGLE to **Vulkan**. Chromium accepts the first Vulkan
ICD it finds and never reconsiders — even when that ICD is **software**
(llvmpipe/lavapipe, or the bundled SwiftShader) and a perfectly good **hardware
OpenGL** path exists alongside it.

That is exactly the situation in a VMware guest:

- **No hardware Vulkan** — the only Vulkan ICDs are software (llvmpipe/lavapipe +
  bundled SwiftShader). `hardwareSupportsVulkan: false`.
- **Hardware OpenGL is present** — VMware SVGA3D via Mesa `svga` exposes GL 4.3 /
  GLES 3.1 with direct rendering.

So Chromium lands on **software SwiftShader**, whose present path stalls
`requestAnimationFrame`; the xterm terminal paints in bursts ("type 10 chars,
pause, dump"), and accelerated WebGL is unavailable (the terminal is stuck on
xterm's slow DOM renderer).

PR #1394 fixed this by **gating on a VMware DMI string match**
(`is_vmware_guest()` reading `/sys/class/dmi/id/*`). That works but is a hack: it
hardcodes a vendor, doesn't generalize to other no-Vulkan-but-has-GL configs, and
describes the *environment* instead of measuring *capability*.

## 2. Goal

Pick the ANGLE backend by a **measured capability precedence**, identical on every
platform, with **no vendor gate**. The precedence we want:

> **hardware Vulkan → hardware GL → software (SwiftShader)**

VMware should then "just work" by falling into the correct rung on its own, while
real GPUs keep Chromium's preferred Vulkan path and headless boxes stay software.

Non-goals: changing Windows/macOS behavior (their defaults are already hardware —
D3D11 / Metal); implementing per-feature blocklist overrides; touching the GPU
process's own init.

## 3. Key insight — ANGLE is never bypassed, only retargeted

Chromium always renders through ANGLE on Linux. This design never disables ANGLE;
it only chooses **which backend ANGLE targets**. Whenever a hardware backend ANGLE
can use is available, that hardware backend runs. The override fires **only** to
correct the one wrong default — accepting *software Vulkan over hardware GL*.

## 4. Design — startup capability probe

Measure two facts once, in the browser process, before CEF GPU init:

| signal | how | meaning |
|---|---|---|
| `has_hw_vulkan` | enumerate Vulkan physical devices (`ash`); true if **any** `device_type != CPU` | DISCRETE/INTEGRATED/VIRTUAL = hardware; llvmpipe/lavapipe/SwiftShader = `CPU` = software |
| `has_hw_gl` | a DRM **render node** exists (`/dev/dri/renderD*`) | a kernel GPU with a render node ⇒ a real GL path (vmwgfx on VMware, i915/amdgpu/nvidia on bare metal) |

### 4.1 Decision table (no platform gate)

| tier | condition | `--use-angle` | `--ignore-gpu-blocklist` | who lands here |
|---|---|---|---|---|
| **HW Vulkan** | `has_hw_vulkan` | *(none — Chromium default)* | no | real GPU (Linux/Win/mac) |
| **HW GL** | `!has_hw_vulkan && has_hw_gl` | `gl` | **yes** | **VMware/SVGA3D**, GL-only GPUs |
| **Software** | neither | *(none — default)* | no | headless / no GPU |

VMware lands in **HW GL** because it *measurably* has no hardware Vulkan but does
have a render node — not because of a vendor string.

### 4.2 Why `--ignore-gpu-blocklist` couples to the HW-GL rung

The blocklist override is a separate concern from backend choice, but it couples
naturally: the HW-GL rung means *"this GPU does hardware GL but Chromium's
preferred path (Vulkan) isn't available here"* — precisely the virtual/unusual-GPU
profile Chromium blocklists WebGL + gpu_compositing for. Having *chosen* HW GL as
the best real path, we tell Chromium to trust it. It is applied **only** on the
HW-GL rung; HW-Vulkan and Software boxes never get it.

Residual risk: a real GPU with HW GL, no Vulkan, and a *legitimate* blocklist
reason (rare). Mitigation/upgrade path in §7.

### 4.3 Authority order

1. **Explicit env** — `AGENTMUX_ANGLE={gl,vulkan,swiftshader,default}` forces the
   backend and short-circuits the probe (sets `--use-angle` only; does not auto-add
   the blocklist override — pair with `AGENTMUX_CEF_EXTRA_FLAGS=--ignore-gpu-blocklist`
   if wanted).
2. **Measured precedence** — the table above.
3. **Chromium default** — when the probe says HW Vulkan or Software.

`AGENTMUX_CEF_EXTRA_FLAGS` (space-separated switches) is always appended last.

### 4.4 Where it runs (process model)

`on_before_command_line_processing` fires per process. We probe **once**, in the
browser process (empty `process_type`), and publish the resolved tier via an
inherited env var (`AGENTMUX_GPU_TIER`) so child processes (gpu/renderer/utility)
apply the same flags **without re-creating a `VkInstance`**. The Vulkan probe is
fully defensive: any load/enumerate failure ⇒ treated as "no HW Vulkan" and we
fall through to the GL check.

### 4.5 Cross-platform

The probe is effectively Linux-only in practice: Windows (D3D11) and macOS (Metal)
report hardware and land in the HW-Vulkan-equivalent top rung untouched. No
`#[cfg]` branching in the decision — the measurement simply returns "hardware
present" where it should.

## 5. Downstream effects (no extra code)

With the HW-GL rung active, `--ignore-gpu-blocklist` un-gates WebGL, and the
existing frontend paths react on their own:

- `termwrap.ts` `detectWebGLSupport()` now succeeds ⇒ xterm auto-selects its
  **WebGL renderer** (`loaded webgl renderer!`) instead of the DOM renderer.
- `gpuutil.ts` / `GpuStatus.tsx` GFX badge probes WebGL ⇒ reads **HW**.

## 6. Implementation plan

1. `agentmux-cef/Cargo.toml`: add `ash` (Vulkan enumeration; the host already
   bundles `libvulkan.so.1`). Linux-only dependency.
2. `agentmux-cef/src/app.rs`:
   - Replace `is_vmware_guest()` with `detect_gpu_tier() -> GpuTier`
     (`HwVulkan | HwGl | Software`), cached + published via env for subprocesses.
   - `has_hardware_vulkan()` (ash) and `has_drm_render_node()` helpers.
   - Rewrite the GPU block in `on_before_command_line_processing` to apply the
     §4.1 table with the §4.3 authority order.
3. Keep `AGENTMUX_ANGLE` and `AGENTMUX_CEF_EXTRA_FLAGS` overrides.

## 7. Verification & upgrade path

**Already verified** (PR #1394, VMware guest): `--use-angle=gl` +
`--ignore-gpu-blocklist` ⇒ `glRenderer = ANGLE(... SVGA3D ... GL 4.3)`,
`skiaBackendType GaneshGL`, `featureStatus.webgl=enabled`, live `getContext('webgl2')`
renders `[0,255,0,255]`, `processCrashCount 0`, terminal on WebGL renderer. This
spec must reproduce **identical** behavior on the same box via the general path
(probe ⇒ HW-GL rung), plus: HW-Vulkan box ⇒ no flags (stays Vulkan); headless ⇒ no
flags (stays software).

**Upgrade path** (future): replace the `has_hw_gl` render-node heuristic with a
real `GL_RENDERER` check (throwaway EGL context, reject software-marker strings —
the `SOFTWARE_MARKERS` list already exists in `gpuutil.ts`) to tighten the
blocklist-override decision on exotic GL-only GPUs.

## 8. Risks

- `--ignore-gpu-blocklist` force-enables features Chromium flagged risky; scoped to
  the HW-GL rung (not global), but a render-node heuristic can over-trust an
  exotic GL stack — §7 upgrade addresses this.
- Adds a `VkInstance` create+enumerate at startup (~tens of ms, once, browser
  process only); fully guarded so any failure degrades to "no HW Vulkan".
