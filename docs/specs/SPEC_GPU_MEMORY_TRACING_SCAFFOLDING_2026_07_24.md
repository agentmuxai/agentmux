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
**Status:** Tier 2 (§2.2) implemented, PR #2294 (2 rounds of real review fixes: UI-thread
marshaling, dev-gating, path confinement — see PR thread). **Run live, same day, after fixing
the separate `task dev` Gap B PATH blocker** (see §6 — that fix was a prerequisite, not part of
this spec, but is what made a live run possible at all). Real capture executed: `begin_gpu_trace`
→ 20 minutes on a live idle-ish dev instance → `end_gpu_trace`, produced an 863 MB real Chromium
trace file. **Result: the scaffolding works end-to-end, but the capture as currently configured
does not contain the data this investigation needs — see §6 for why and what's next.** Not a
failure of the code; a real, evidenced limit of the exposed CEF API surface, documented so nobody
re-discovers it by repeating the same 20-minute wait.

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
AGENTMUX_CEF_EXTRA_FLAGS="trace-startup=-*,disabled-by-default-memory-infra trace-startup-file=C:\path\to\trace.json trace-startup-duration=1800"
```
**Space-separated, not semicolon-separated** — `on_before_command_line_processing`
(`agentmux-cef/src/app/mod.rs`) parses this env var with `.split_whitespace()`, then `=` splits
each token into switch/value (verified against the actual parsing code, not assumed — an earlier
draft of this recipe used semicolons, which that parser doesn't recognize as a separator, so it
silently produced no trace file). Corollary: the output path can't contain a space, since
`split_whitespace()` doesn't respect quoting — pick a path like `C:\Users\<you>\trace.json`, not
one under a directory with a space in its name.

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
per the reasoning in the file's own doc comment) and `end_gpu_trace` (requires a `filename` arg —
a bare file name, not a path; see below — wraps completion in a `GpuTraceEndCallback` via
`wrap_end_tracing_callback!` that logs the flushed file path + size via `tracing::info!` once
CEF's callback fires). Wired into `ipc.rs`'s dispatch table next to
`toggle_devtools`/`inspect_element_at`, and re-exported from `commands/window/mod.rs` the same way
every sibling module in that directory is. `cargo check -p agentmux-cef` is clean — zero warnings
or errors in the new file.

**Post-review hardening (reagentx `CHANGES_REQUESTED`, PR #2294):** the first pass shipped both
commands reachable unconditionally in every build and let `end_gpu_trace` write to any
caller-supplied absolute path. Both fixed before merge:
- Both commands now call `require_dev_mode()` first — a runtime `AGENTMUX_DEV=1` check (the same
  pattern `app/mod.rs`'s GPU-tier switch already uses), not a build-identity check. No legitimate
  production caller for a diagnostics-only tracing toggle.
- `end_gpu_trace`'s arg changed from an arbitrary `path` to a `filename` — rejected outright if it
  contains a path separator or `..`/`.`. The real output path is always
  `<instance data dir>/gpu-traces/<filename>` (`resolve_trace_path`, creates the subdirectory if
  missing). This confines the write target *by construction* rather than validating an
  attacker-suppliable absolute path after the fact.

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

## 6. Live run result (2026-07-24) — scaffolding works, capture is missing the payload data

**Prerequisite fixed first:** `task dev` was separately blocked all session by a Gap B PATH bug
(`bash: executable file not found` deep inside go-task's `build:host:windows` step) — root cause
turned out to be a cmd.exe quoting bug (`set "PATH=...;%PATH%"`, quoted, silently failed to
actually update the environment variable; `set PATH=...` unquoted works). Not part of this spec,
but the reason a live run was possible at all this session.

**The run:** `begin_gpu_trace` (default categories) → confirmed `gpu_trace: started` in the srv
log → left running 20 minutes on an otherwise-idle dev instance → `end_gpu_trace` → confirmed
flush, 863 MB trace file written to `<data dir>/gpu-traces/`.

**The problem:** the file contains real `GlobalMemoryDump` events (70 of them — periodic dumps
genuinely fired, roughly every ~30s, matching Chromium's default background interval) but
**zero** allocator/size payload data (`grep -c '"size"'` → 0, `grep -c "allocator"` → 0). Each
dump's own args explain why: `"dump_type":"summary_only","level_of_detail":"background"`.
Chromium's **background-level** dumps (the ones a bare category-filter string triggers by
default) are deliberately near-zero-cost telemetry — they record *that* a dump happened, not the
per-category size breakdown. That breakdown only exists in **detailed**-level dumps, which
require an explicit `memory_dump_config.triggers[].mode: "detailed"` — normally set via a full
JSON `TraceConfig`, not the simple comma-separated category-filter string.

**Tried and empirically ruled out:** passing a full JSON `TraceConfig` string (with
`memory_dump_config`) as `begin_gpu_trace`'s `categories` argument, on the theory that Chromium's
`TraceConfig` constructor auto-detects JSON vs. category-filter shorthand from the same string
parameter. Tested live with a short (40s) capture: the resulting trace had **no**
`GlobalMemoryDump` events at all and no `disabled-by-default-memory-infra` events either — the
JSON string was very likely parsed as a (nonsensical, matching nothing) category filter, not
recognized as a config object. Also confirmed by direct binding search: `cef::begin_tracing`'s
signature (`categories: Option<&CefString>, callback: Option<&mut CompletionCallback>`) has no
separate parameter for a dump-trigger config, and the vendored crate source has no
`RequestGlobalMemoryDump`-equivalent or richer `TraceConfig`-accepting entry point at all —
checked directly against the exact resolved crate version, not assumed.

**This is a real, hard limit of the exposed CEF API surface**, not a bug in `gpu_trace.rs` — the
scaffolding correctly does everything `cef::begin_tracing`/`cef::end_tracing` support; those two
functions just don't support requesting detailed dumps.

### What would actually work (not implemented — next scoped step for whoever continues this)

1. **Check whether CEF exposes a lower-level tracing API** beyond `begin_tracing`/`end_tracing` —
   e.g. a way to submit a raw Perfetto `TraceConfig` protobuf, or a Chromium IPC/mojo interface
   for `RequestGlobalMemoryDump` with an explicit `DETAILED` level. This may require going below
   CEF's public API into Chromium internals CEF doesn't wrap, which likely isn't feasible from
   `agentmux-cef` at all without patching CEF itself — worth a quick check before ruling it out,
   but treat "not possible without a CEF patch" as a live, real possibility.
2. **Chromium's own `--trace-startup-file` command-line path (Tier 1) may behave differently** —
   it goes through `TracingController::StartTracing` with a config built from
   `base::trace_event::TraceConfig::TraceConfig(category_filter_string, trace_option)`, which is
   a different code path than `cef_begin_tracing`'s. Worth testing Tier 1 specifically (not yet
   done — this session only validated Tier 2) before concluding detailed dumps are unreachable
   from CEF entirely.
3. **Fall back to a non-tracing measurement**: if detailed GPU memory-infra dumps prove
   unreachable via any CEF-exposed path, the investigation may need a different instrument
   entirely — e.g. Windows' own ETW GPU provider (`Microsoft-Windows-DxgKrnl`), or a
   vendor-specific tool (PIX, Nsight), run externally against the GPU process PID rather than
   through CEF/Chromium's own tracing at all. This sidesteps the CEF API limitation completely at
   the cost of losing Chromium's own category/allocator semantics.

The 863 MB (and a smaller ~79 KB failed-JSON-config) capture files are left in
`<dev data dir>/gpu-traces/` in case someone wants to double-check this reading of the data
before pursuing (1)-(3).
