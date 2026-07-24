# Spec: GPU Memory Tracing Scaffolding — a real trace, not another process-level guess

**Date:** 2026-07-24
**Motivating issue:** #2218 (reopened) — `docs/status/STATUS_PF_COMMIT_GROWTH_INVESTIGATION_2026_07_24.md`
§9-§12 established, from ~9 hours of real telemetry on a single idle-ish pane, that system commit
grows ~0.8 GB/hour **independent of renderer count** (ruled out by the July 16 restart experiment)
**and independent of logged content/activity** (ruled out §12, same session). Neither of the two
obvious explanations holds. The next step that can actually narrow this further is a real GPU
memory trace during confirmed-idle hours — process-level counters (`Private Bytes`, `Committed
Bytes`) cannot see inside the GPU process's own allocator, which is exactly where the
"unattributed" bucket lives.
**Status:** Tier 2 (§2.2) implemented same-day — `agentmux-cef/src/commands/window/gpu_trace.rs`,
`begin_gpu_trace`/`end_gpu_trace` RPC commands, `cargo check -p agentmux-cef` clean. Tier 1 (§2.1)
is still just a documented recipe — no code needed there, nothing to implement. Neither tier has
been run live yet (`task dev` on this branch is separately blocked — unrelated Gap B PATH issue,
see conversation history — so this is verified by typecheck only, not a live capture).

---

## 1. What we're trying to see

Every measurement so far (this investigation, and the July 2 / July 16 retros before it) treats
the GPU/driver-committed memory as a black box: a number that grows, attributed to "System" by
subtraction, with no visibility into *what kind* of GPU allocation is growing. Chromium's own
`memory-infra` tracing system is the tool built for exactly this — it breaks GPU memory into
named categories instead of one opaque total:

| Category | What it tracks | Source |
|---|---|---|
| `gpu` (in the GPU process) | **All GPU allocations, size column** — the authoritative total regardless of which process references it | [probe-gpu.md](https://chromium.googlesource.com/chromium/src/+/lkgr/docs/memory-infra/probe-gpu.md) |
| `cc` (per renderer/browser process) | Chrome Compositor resource allocations — become GPU allocations under GPU rasterization | same |
| `skia/gpu_resources` | GPU resources used by Skia (AgentMux's UI is Skia-rendered) | same |
| `GPUMemoryBuffer` (per process) | Active GPU memory buffers in that process | same |

Shared allocations (SharedImages, GMBs referenced from multiple processes) report their real
`size` only in the *owning* process/category; everywhere else `effective size` reads 0 to avoid
double-counting — so the `gpu` category's `size` column in the GPU process is the number to trust
for a true total, matching what "unattributed" is measuring today by subtraction.

**Goal: capture the `gpu` category (plus `cc`/`skia/gpu_resources`/`GPUMemoryBuffer` for
attribution) across several idle hours and see which one actually grows.** If none of them grow
while system commit still does, the leak is somewhere memory-infra doesn't instrument at all
(driver-internal WDDM paging structures, ANGLE's own D3D11 layer) — which would itself be a
useful, narrower negative result.

## 2. Two tiers — a same-day manual experiment, then real scaffolding

### 2.1 Tier 1 — zero code changes, can be run today

AgentMux already has an escape hatch for exactly this kind of A/B: `AGENTMUX_CEF_EXTRA_FLAGS`
(`agentmux-cef/src/app/mod.rs`, `on_before_command_line_processing` — "lets us A/B GPU/compositor
flags"). Chromium's own startup-tracing mechanism is pure command-line:

```
--trace-startup=-*,disabled-by-default-memory-infra
--trace-startup-file=<path>.json
--trace-startup-duration=<seconds>
```

(syntax: [memory_infra_startup_tracing.md](https://chromium.googlesource.com/chromium/src/+/112.0.5615.165/docs/memory-infra/memory_infra_startup_tracing.md))

**Caveat found during research, not yet verified against this exact CEF/Chromium build:**
`--trace-startup-duration` is documented with short examples (single-digit seconds) — unclear
whether it accepts a value large enough for a multi-hour idle-session capture (e.g. `32400` for
9h), or whether the startup-tracing path is only intended for short boot-time traces. **First
concrete step: try it with a moderate value (e.g. 1800s / 30min) as a dry run before trusting it
for a full multi-hour capture.** If it doesn't hold up, tier 1 is still useful as a *short*
before/after snapshot (e.g. 5 minutes idle vs. 5 minutes right after opening the pane) even if it
can't cover the full multi-hour window tier 2 is built for.

Launch with:
```
AGENTMUX_CEF_EXTRA_FLAGS="trace-startup=-*,disabled-by-default-memory-infra;trace-startup-file=C:\path\to\trace.json;trace-startup-duration=1800"
```
(matches the existing `k=v` parsing already in `on_before_command_line_processing` for this env
var — semicolon-separated, `=` splits switch from value).

Output is a Chrome trace JSON, viewable in `chrome://tracing` (load file) or
[Perfetto UI](https://ui.perfetto.dev) — group by process, look at the GPU process's `gpu`
category memory-infra dump entries over time.

### 2.2 Tier 2 — a real trigger command, for an arbitrary-length idle-hours capture

Tier 1 is bounded by whatever `--trace-startup-duration` turns out to actually support, and
requires restarting AgentMux with the flag set *before* the idle period starts. For a real
multi-hour, operator-controlled capture (start it, then just leave the pane alone for however
long, stop it when done), the better fit is CEF's runtime tracing API:

- `CefBeginTracing(categories, callback)` — starts tracing with an explicit category filter.
- `CefEndTracingAsync(tracing_file, callback)` — flushes every process's trace data to disk;
  callback fires once complete.
([cef_trace.h reference](https://cef-builds.spotifycdn.com/docs/114.2/cef__trace_8h.html))

**Implementation-time unknown from the first pass of this spec — now resolved.** Direct
verification against the exact resolved crate version (`cef = "148.3.0+148.0.9"`, checked in
`Cargo.lock`, source read from the local Cargo registry cache rather than guessed) confirmed both
functions exist and are re-exported at the crate root: `cef::begin_tracing(categories:
Option<&CefString>, callback: Option<&mut CompletionCallback>)` and `cef::end_tracing(tracing_file:
Option<&CefString>, callback: Option<&mut EndTracingCallback>)`, plus the `EndTracingCallback`
type and a `wrap_end_tracing_callback!` macro for implementing a custom completion handler (no
prior usage of that macro pattern existed anywhere in `agentmux-cef` — this is the first).

**Implemented**, modeled directly on the existing `toggle_devtools` handler
(`agentmux-cef/src/commands/window/meta.rs:333-337` — same signature shape, same
`state`/`args`/`Result<Value, String>` contract, dispatched the same way through `ipc.rs`):
`agentmux-cef/src/commands/window/gpu_trace.rs` — `begin_gpu_trace` (guards against overlapping
calls with a module-local `AtomicBool`, defaults to the categories listed in §1, fire-and-forget
per the reasoning in the file's own doc comment) and `end_gpu_trace` (requires a `path` arg, wraps
completion in a `GpuTraceEndCallback` via `wrap_end_tracing_callback!` that logs the flushed file
path + size via `tracing::info!` once CEF's callback fires). Wired into `ipc.rs`'s dispatch table
next to `toggle_devtools`/`inspect_element_at`, and re-exported from
`commands/window/mod.rs` the same way every sibling module in that directory is. `cargo check -p
agentmux-cef` is clean — zero warnings or errors in the new file.

## 3. Long-duration capture — avoiding an unbounded trace file over multiple hours

A full-event trace (every paint, every allocation) over 9 hours would be enormous and largely
useless noise for this investigation — we only care about periodic memory *snapshots*, not
frame-by-frame event timing. Chromium's memory-infra supports exactly this distinction via
**periodic dump level**:

- **BACKGROUND level** — "designed to have almost no impact in execution, running very fast" —
  intended to be safe to leave on continuously (this is literally what production Chrome uses for
  its own telemetry). **This is the level to use for the multi-hour capture window.**
- **DETAILED level** — full breakdown, expensive, meant for short manual investigation, not
  multi-hour background capture.

Category string alone (as used in tier 1/2 above) may default to a reasonable periodic interval;
if finer control turns out to be necessary, Chromium's advanced trace-config JSON supports an
explicit `periodic_interval_ms` per dump level (documented example: `50` for light/background,
`1000` for detailed) — pass via `--trace-config-file` for tier 1, or check whether
`CefBeginTracing` accepts a full config object (vs. just a category string) for tier 2. **Pick a
periodic interval on the order of tens of seconds to a few minutes for a multi-hour background
capture** — frequent enough to see the ~0.8 GB/hour trend clearly (this investigation's own
telemetry sampled every 10-13s and that was more than sufficient resolution), infrequent enough
that hours of dumps stay a reasonably sized file.

## 4. What to actually look for once a trace exists

1. Open the resulting JSON in `chrome://tracing` or [ui.perfetto.dev](https://ui.perfetto.dev).
2. Find the GPU process's `gpu` category memory-infra dumps; plot `size` over the capture window.
3. **If `gpu` category size tracks the ~0.8 GB/hour growth this investigation measured** — the
   leak is a genuine, instrumented GPU allocation category. Check the top-level allocator
   breakdown within that dump (command buffer, texture pool, shared images) to narrow further —
   that becomes the next concrete lead, likely mappable to a specific Chromium bug class from
   `STATUS_PF_COMMIT_GROWTH_INVESTIGATION_2026_07_24.md` §11's research (SharedImage/GMB handles).
4. **If `gpu` category size stays flat while system commit still grows** — memory-infra doesn't
   see this allocation at all, meaning it's driver-internal (WDDM paging buffers, ANGLE D3D11
   layer state) below Chromium's own instrumentation. That's a materially different, harder
   problem — likely needs a GPU-vendor-specific tool (PIX, Nsight, or Windows' own ETW GPU
   provider) rather than anything Chromium-side. Worth knowing either way; changes what the next
   spec after this one should even attempt.

## 5. Explicitly out of scope for this spec

- Not proposing `--disable-gpu` as a mitigation anywhere in this document — already rejected,
  owner policy, July 16 retro §5 item 3. This spec is purely about *seeing* the leak, not fixing
  it yet.
- Not committing to which tier ships first — tier 1 is a today-sized experiment; tier 2 is real
  feature work (new Rust command, IPC wiring, testing) that should get its own implementation
  pass and PR once tier 1 (or direct verification of the CEF binding surface) says it's worth
  building.
- Not designing the eventual mitigation (§11's "proactive renderer recycle" idea from the status
  doc) — that's downstream of knowing what the trace actually shows.
