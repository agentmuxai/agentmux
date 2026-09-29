# Retro: AgentMux launched with an invisible window when the VM's GPU had no 3D (2026-09-29)

**Status:** retro. The fix is proposed in §7; nothing has shipped yet.
**Host:** charlie, an Ubuntu 26.04 VMware Workstation guest on gamerlove, running AgentMux 0.58.2
(`local-main-b28b7a-fef1acbd`).
**Symptom:** after launch, AgentMux had a taskbar entry but no window. It stayed that way, with no
error, no fallback and no message.
**Operator's standard, which this violated:** "it should never just not show", and "these issues
were once already addressed". The second point is correct. Several earlier fixes each promised to
prevent this class of failure, and §4 shows where each one fell short.

---

## 1. Timeline (UTC)

| Time | Event | Evidence |
|---|---|---|
| 09:46:46–09:48:40 | gamerlove's NVIDIA driver (RTX 5070) resets repeatedly: `nvlddmkm` event 14, then event 153 (TDR) six times | gamerlove System event log |
| 09:48:55 | VMware Workstation's SVGA renderer loses the host GPU: `unrecoverable error: (svga)`, `Lost connection to mksSandbox`. The charlie VM halts. The Opaz agent and the CEF 154 Linux build die with it | charlie's `vmware.log` |
| ~12:37 | Noticed (Opaz silent); diagnosed over SSH | this session |
| 12:42–12:52 | gamerlove is cleaned up and rebooted. To stop a host GPU reset from killing the VM again, **AgentY recommended turning off the VM's "Accelerate 3D graphics"**, which the operator did (`mks.enable3d = "FALSE"`) | `.vmx` |
| 13:01:35 | AgentMux 0.58.2 starts on charlie. The log says `resolved GPU tier for ANGLE selection: hw-gl` | `agentmux-host-v0.58.2.log` |
| 13:01:36–37 | The Chromium GPU process crashes three times: `GPU process exited unexpectedly: exit_code=8704` | `cef-debug.log` |
| 13:01:41.355 | `[startup-paint] revealing gated window … reason: timeout, elapsed_ms: 4001`. No first-paint signal ever arrives. A frameless, transparent window that has never painted is "shown", which means it is invisible | host log |
| 13:04 | Relaunched with `AGENTMUX_ANGLE=swiftshader`. The window is revealed on timeout at 13:04:52 and the first paint arrives at 13:04:54. It is visible, with no GPU crashes | host log, `cef-debug.log` |

There was no data loss. Only charlie's own work was lost to the VM crash: the Linux build, about 3 hours.

## 2. What happened, layer by layer

**Layer 1: the GPU tier was guessed, not measured.**
- `detect_gpu_tier()` (`agentmux-cef/src/app/gpu.rs:50-67`) picks `HwGl` when there is no hardware Vulkan but a DRM render node exists (`has_drm_render_node`, `gpu.rs:103-110`).
- VMware's `vmwgfx` exposes `/dev/dri/renderD128` whether or not 3D is enabled. So a guest with 3D turned off, which has no working hardware GL, was classed as hardware GL.

**Layer 2: the hardware-GL tier disables Chromium's own safety net.**
- `HwGl` maps to `--use-angle=gl --ignore-gpu-blocklist` (`agentmux-cef/src/app/mod.rs:757-777`).
- Chromium's blocklist and its SwiftShader fallback are exactly what would have rescued an unusable GPU. The `--enable-unsafe-swiftshader` safety net (`mod.rs:~1002`, the 2026-06-11 analysis) assumes Chromium is allowed to decide. The forced `use-angle=gl` together with `ignore-gpu-blocklist` took that decision away.

**Layer 3: nothing notices "alive but never painted".**
- The paint gate (`agentmux-cef/src/client/navigation.rs`, `PAINT_GATE_SAFETY_TIMEOUT_MS = 4000`) is documented as a "backstop against a genuinely stalled renderer" (`navigation.rs:17-48`). On timeout it calls `window.show()` (`reveal_gated_window` → `finish_gated_reveal` → `try_show_resolved_window`) and dismisses the splash. It doesn't ask *why* no paint came, and it doesn't check whether the GPU process died.
- Because the window is transparent (`Settings.background_color: 0x00000000`, `agentmux-cef/src/lib.rs:1080`, plus a frameless delegate), a window that never painted doesn't look blank or white. **It doesn't appear at all.**
- The host process stayed up, so no exit-based recovery ran. The launcher's `--disable-gpu` retry only fires when the host exits abnormally twice with the same code (`agentmux-launcher/src/supervisor/unix.rs:699-710`).

**Layer 4, the trigger: the environment changed under an unverified assumption.**
Turning off 3D was a reasonable way to protect the VM from host GPU resets. But nobody, including AgentY who recommended it, checked what AgentMux's GPU tier logic would do on a VM without 3D. The spec that introduced the tiers had been tested only with 3D on (§4 below).

## 3. Why the fixes that already existed didn't catch it

| Earlier work | What it promised | Why it missed this |
|---|---|---|
| **SPEC_LINUX_GPU_BACKEND_PRECEDENCE_2026_06_13** (#1394) | Tiers: hardware Vulkan, then hardware GL, then software. §4.2 and §8 name the risk ("a render-node heuristic can over-trust an exotic GL stack"). §7's upgrade path replaces the heuristic with a real `GL_RENDERER` check that rejects software renderers | **§7 was never built.** `gpu.rs:100-101` still labels it future work. It was tested only on a VMware guest with 3D on ("SVGA3D … GL 4.3") |
| **CRASH_GPU_PROCESS_FATAL_2026_05_20** | §1: "the user must never see a crash … every fault becomes an invisible, sub-second auto-recovery". §4.3: (a) `--disable-gpu-process-crash-limit`; (b)/(c) after N GPU crashes, relaunch the host with `--disable-gpu` | (a) was never implemented. (b)/(c) was implemented only for **host exits**, in the launcher ladder. Nothing counts GPU-process crashes while the host stays up |
| **gpu-crash-recovery.md** (2026-04-04, archived as "shipped") | Layer 1: a persistent `gpu-health.json` crash counter, so the next launch starts degraded | `gpu_health` appears nowhere in the code. A second launch on charlie would have failed the same way. The "shipped" label is wrong |
| **SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20** | §8: "'process exited' is not enough"; health must mean responsive, not just alive | Only renderer hang detection (§8.1) was built. There is no "GPU dead" or "never painted" health signal |
| **Paint gate** (#2151, then #2968, whose title reads "…so the window never shows blank") | The timeout is only a backstop, and "if this timeout is ever observed firing … that is the signal a dedicated pass is now overdue" | It fired here, and nothing records or reports when it fires. On timeout it reveals a window that hasn't painted, without checking. On Linux with a dead GPU, "never shows blank" became "never shows" |
| **RETRO_MAIN_WINDOW_SHOW_SILENT_NOOP_2026_08_13** (#2567) | Added a retry for `show()`. The retro says it's "not a guarantee it can never recur" | A different cause: `show()` did run here; the window just had nothing drawn in it |

**The pattern:** each earlier incident was fixed at the layer where it surfaced (flag selection,
host-exit retries, `show()` retries). Each postmortem wrote down the next layer as a follow-up, and
none of those follow-ups were tracked to completion. Three of them (§7's `GL_RENDERER` check,
counting GPU crashes while the host stays up, and a persistent GPU-health record) would each have
prevented this failure on their own.

## 4. What should be true (invariants)

1. **I1: a window is always visible and usable.** If the app is running, the user either sees a
   usable window, or sees an explicit message and a working fallback. "Nothing" is never an
   acceptable outcome.
2. **I2: GPU acceleration is an optimization, never a requirement.** Any GPU failure degrades to
   software rendering on its own, within the same launch.
3. **I3: never override Chromium's GPU safety net without checking first.** A flag that disables the
   blocklist or forces a backend must only be passed after checking that the backend is real.
4. **I4: backstops announce themselves.** When a safety timeout fires, it's recorded somewhere a
   human will see it, not just in a log line.

## 5. What we got wrong in process

- Promised follow-ups lived only inside the specs, with no tracking issue and no owner. So "the
  §7 upgrade path" stayed "future" for three and a half months.
- A doc was archived as "shipped" when only part of it had shipped (`gpu-crash-recovery.md`).
- The GPU tier logic was tested only on the configurations it was designed for (3D on). No test ran
  it in a VM with 3D off, or with no GPU at all.
- The environment change (turning off 3D) was recommended without checking which of AgentMux's own
  assumptions depended on it.

## 6. Recovery used today

Relaunched once with `AGENTMUX_ANGLE=swiftshader`, an existing override that skips the tier logic.
There was no persistent config change on charlie. Without the code fix, the next normal launch on
charlie will be invisible again.

## 7. Fix plan (each item maps to an invariant)

| # | Change | Invariant |
|---|---|---|
| F1 | **Measure GL instead of guessing:** before choosing `HwGl`, run a throwaway EGL context (surfaceless, in a short-lived child process so Mesa never loads into the browser process) and read `GL_RENDERER`. Take `HwGl` only for a real hardware renderer; `llvmpipe`/`softpipe`/`swrast`/`SwiftShader`/a failed probe mean `Software`. This is SPEC §7, finally built | I2, I3 |
| F2 | **Only pass `--ignore-gpu-blocklist` when F1 has confirmed hardware GL.** Every other tier leaves Chromium's fallback alone | I3 |
| F3 | **Detect "alive but not painting":** if the paint gate times out and the GPU process has crashed during startup, relaunch the host once with software rendering (`--disable-gpu`, or `use-angle=swiftshader`). Record that on disk (a real `gpu-health` record, the Layer 1 that was never built) so the next launch starts in software until a later hardware probe passes | I1, I2 |
| F4 | **A window that has never painted must not be transparent:** until the first real frame arrives, show an opaque surface in the app's background colour ("Starting…"), so the timeout can never produce nothing | I1 |
| F5 | **Make backstops visible:** when the paint-gate timeout fires, record it (the startup-bench and diagnostics log) and show it in the app's diagnostics. Also correct `gpu-crash-recovery.md`'s "shipped" status | I4 |
| F6 | **Tests:** unit-test the tier decision (render node without hardware GL gives `Software`; `llvmpipe` gives `Software`; SVGA3D gives `HwGl`). Also run a manual matrix on a VMware guest with 3D on, 3D off, and on a host with no DRM, with the result recorded in the spec | all |
| F7 | **Track every retro and spec follow-up as a GitHub issue with an owner**, linked from the doc. A doc can't be marked "shipped" or "implemented" while any of its follow-ups is still open | process |

F1 and F2 are the direct fix for this incident, and they ship first. F3 and F4 make the whole class
of failure (any GPU death, on any platform) survivable, and they're next.
