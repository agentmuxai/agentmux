# SPEC: macOS Launch Speed + Splash Load-Time Telemetry

**Date:** 2026-07-02
**Status:** Proposed (analysis only — no code changes yet)
**Area:** Launcher (macOS, cross-referenced against Windows/Linux)
**Prior art:** `SPEC_LAUNCHER_MACOS_DEV_INTEGRATION_2026_05_30.md`, `SPEC_SPLASH_STARTUP_TELEMETRY_2026_06_25.md` (Windows), `SPEC_SPLASH_TELEMETRY_LINUX_2026_06_27.md`, `SPEC_MACOS_LAUNCH_COHERENCE_2026_06_18.md`, `SPEC_MACOS_PACKAGING_2026_05_30.md`, `docs/CEF_ARCHITECTURE.md`

---

## 0. Ask

Two-pronged: (A) make macOS app launch as fast as possible, removing any blocking/slow steps, using Windows and Linux as the comparison baseline; (B) surface per-phase load times live in the splash screen. This document is the analysis + design; no implementation is included here.

**Correction to CLAUDE.md before anything else:** CLAUDE.md's Build System section states *"On Linux/macOS, `task dev` still invokes the host directly (Phase 7 cross-platform parity will integrate the launcher)."* This is stale. `SPEC_LAUNCHER_MACOS_DEV_INTEGRATION_2026_05_30.md` (Phase 1) landed and is implemented: `Taskfile.yml:754-794` (`dev:serve`) and `agentmux-launcher/src/main.rs:374-383` both confirm the launcher unconditionally owns srv+host spawn on macOS/Linux today. `task dev:standalone` is the only bypass, and it's an explicit debug escape hatch (`Taskfile.yml:1078-1096`), not the default. **Practical implication:** macOS dev-mode timing is representative of packaged-build timing (same code path) — a fast/instrumented dev loop is a valid proxy for real launch performance, and this doc's CLAUDE.md correction should ship alongside whatever change lands from this spec.

---

## Prong A — macOS Launch Speed

### A.1 Current macOS launch timeline (file:line)

1. `main()` (`agentmux-launcher/src/main.rs:94`) — `suppress_os_crash_dialogs()` is a no-op on macOS.
2. **Splash paints on the main thread; supervisor runs on a worker thread** (`main.rs:105-143`) — deliberate: AppKit must own the main thread, so `launcher_main` (srv+host supervision) runs on a separate OS thread with its own Tokio runtime while the main thread pumps the splash's CF runloop. This is a hard constraint (§A.3), not a bottleneck — splash construction itself is cheap (in-process AppKit calls + one embedded-PNG decode).
3. `launcher_main` (`main.rs:225-384`), still pre-supervisor:
   - `find_cef_binary()` (`agentmux-launcher/src/binary_resolution.rs:8-47`) — synchronous, **uncached** `read_dir` scan every launch to locate a binary whose versioned name is knowable at compile time.
   - `std::fs::canonicalize` ×2 + `Path::exists()` for the self-spawn guard (`main.rs:299-312`) — additional synchronous syscalls.
4. `supervisor::run_unix` (`agentmux-launcher/src/supervisor/unix.rs:62-579`) — shared with Linux:
   - Data-dir resolve/create, IPC socket dir setup (`unix.rs:76-128`) — synchronous.
   - `bind_socket_with_recovery` (`unix.rs:157`) — Unix-domain single-instance bind with `flock`-guarded stale-socket recovery (non-atomic, more code than Windows' pipe bind, but not a meaningful latency cost).
   - **`saga::compensate_unresolved_launcher_sagas` + vacuum, `.await`ed synchronously** (`unix.rs:213-239`) — before any spawn begins. Bounded by SQLite saga-log size (normally tiny), but currently sequential with no ordering dependency forcing that.
   - **`srv_spawner::spawn_srv(...).await`, fully blocking** (`unix.rs:289-297`) — supervisor does not proceed to host spawn until srv prints `AGENTMUXSRV-ESTART` on stderr or a 30s timeout fires (`srv_spawner.rs:470-501`).
   - **Only after srv signals ready** does `spawn_host_unix` run (`unix.rs:329-338`, `host_spawn.rs:111-179`).
5. Host process pre-paint work (`agentmux-cef/src/lib.rs:146-330`), macOS-specific:
   - Seatbelt sandbox init for subprocess roles, before CEF framework load (`lib.rs:239-258`) — required ordering per CEF's own model.
   - **Explicit CEF framework `dlopen`** via `LibraryLoader` (`lib.rs:262-278`) — loads the whole multi-hundred-MB Chromium Embedded Framework at runtime; almost certainly the single largest fixed cost in the timeline, and it's inherent to CEF-on-macOS (§A.3), not something to "fix."
   - `resolve_browser_subprocess_path()` for the dedicated Helper.app (`lib.rs:83-100`), path joins, several cheap native AppKit setup calls (menu bar, dock icon, reopen handler, accessibility governor, drag-slideback) — each individually cheap, executed serially.
6. First paint — host writes `AGENTMUX_SPLASH_READY_FILE` from `on_load_end`; splash's poll loop (`splash_mac.rs:371`, 8ms `std::fs::metadata` poll) picks it up, then a 160ms fade (`FADE_OUT`, `splash_mac.rs:66`) before `orderOut:`.

**Sequential/blocking today:** binary resolution scan → path canonicalization → data-dir setup → socket bind → saga recovery + vacuum → **srv spawn + ESTART wait (up to 30s)** → host spawn → host's own sandbox-init/dlopen/CEF-init chain → first paint.

**Already async/off critical path:** event-log disk writer, saga coordinator task, IPC accept loop, srv's stdout/stderr forwarding, splash's render loop (separate thread from supervisor).

### A.2 Comparison vs Windows and Linux

| Step | Windows | macOS | Linux |
|---|---|---|---|
| srv→host sequencing | Sequential (ESTART-gated) | Sequential (ESTART-gated) | Sequential (ESTART-gated) |
| Splash typed-progress feed | Yes (`splash.rs`) | **No — `rx` dropped** (`unix.rs:204-208`) | Yes (`splash_linux::spawn(rx)`) |
| Suspend+job-assign+resume dance | Yes, both children (`srv_spawner.rs:328-387`, `host_spawn.rs:14-95`) | Not needed (no unassigned-child window) | Not needed |
| Kernel parent-death guarantee | Job Object `KILL_ON_JOB_CLOSE` | **None** — tokio-side wait loop + SIGTERM→grace→SIGKILL only | `PR_SET_PDEATHSIG` |
| Single-instance bind | Atomic named-pipe bind | `flock`-guarded recovery (more code, same-ish cost) | Same as macOS |
| Extra native chrome pre-paint | No | Seatbelt sandbox, CEF `dlopen`, menu bar, dock icon, reopen handler, accessibility governor | No |

**The srv→host sequential gate is universal — not macOS-specific.** This is the single biggest opportunity and applies to all three platforms equally (§A.4.1).

macOS-only overhead beyond that gate: the CEF-framework `dlopen` (hard constraint, §A.3) and a battery of small native AppKit setup calls (individually cheap). macOS also has the **weakest** parent-death guarantee of the three platforms (no Job Object, no `PR_SET_PDEATHSIG`) — a reliability gap worth tracking separately, not a speed issue.

### A.3 Hard constraints — explicitly out of scope for "fixing"

- **Splash-on-main-thread / supervisor-on-worker-thread split** (`main.rs:105-143`) — the only way to paint anything before CEF's multi-second init; AppKit requires main-thread ownership. Do not "simplify" this away.
- **CEF framework must be `dlopen`'d at runtime, and Seatbelt sandbox init must precede it** (`lib.rs:239-278`) — CEF's own documented requirement (`docs/CEF_ARCHITECTURE.md:963-965`). This dlopen is likely the largest fixed cost on macOS and is not addressable from the launcher side.
- **Renderer/GPU/utility subprocesses must run as a separate "AgentMux Helper.app," not a self-re-exec** (`lib.rs:76-100`) — the macOS process-identity model rejects re-execing the main bundle binary for subprocess roles (would crash-loop).
- **Per-version `CFBundleIdentifier`** (`SPEC_MACOS_LAUNCH_COHERENCE_2026_06_18.md` §4) — required for correct LaunchServices routing across multiple installed versions; bookkeeping cost at package time, not launch time.
- **Reopen handler installed via raw ObjC runtime calls** (`splash_mac.rs:203-282`) — CEF re-registers its own Apple-Event handler, so the naive `NSAppleEventManager` hook is inert; this workaround is load-bearing for Finder/Dock double-click UX.
- **Notarization/Gatekeeper/codesign** (`SPEC_MACOS_PACKAGING_2026_05_30.md`) — an unconditional, OS-cached, one-time launch gate for packaged builds. Not a per-launch cost, not optimizable, must not be trimmed.

### A.4 Ranked opportunities (macOS, with cross-platform applicability noted)

1. **~~Parallelize srv spawn and host spawn~~ — investigated, blocked by a real dependency, not implemented.** `host_env` is built from `SrvSpawnResult` (`srv_spawner.rs:42-58`), specifically `ws_endpoint`/`web_endpoint`/`instance_id` parsed out of the `AGENTMUXSRV-ESTART` stderr line (`srv_spawner.rs:404-441`) — srv self-binds an OS-assigned ephemeral port and reports it back; the launcher never passes `--port` (confirmed: no such arg in `spawn_srv`). This is intentional (CLAUDE.md: "each instance spawns its own srv on a dynamic port — no port conflicts" across parallel-running instances), not an oversight. Since env vars must be set before `Command::spawn()`, the host genuinely cannot be spawned before this data exists — today's sequential gate is load-bearing for the current port-discovery design, not just a missed optimization. (`auth_key`, by contrast, *is* launcher-generated upfront via `uuid::Uuid::new_v4()` at `srv_spawner.rs:292`, before srv is even spawned — only the port/endpoint data is the blocker.) Two ways to actually fix this exist but are materially bigger changes than a reorder: (a) have the launcher pre-allocate/bind the port itself and pass it into both srv and host, touching `agentmux-srv`'s own bind logic; or (b) switch host bootstrap from env-vars to an IPC-push (host spawns immediately, connects to the launcher's already-open socket, and receives `ws_endpoint`/`auth_key` once srv reports them), a protocol change to `agentmux-cef`'s startup path. Both need their own design spec and cannot be safely verified without a full GUI-driven multi-instance test pass. **Decision (2026-07-03): deferred — not pursued in this implementation stack.** Superseded as the "biggest opportunity" by item 5 below, which is safe and has no such dependency.
2. **~~Cache/derive the host binary path~~ — investigated, turned out to already be a non-issue.** `binary_resolution.rs`'s `find_cef_binary()` already checks the versioned path directly first (`runtime_dir.join(format!("agentmux-{}", env!("CARGO_PKG_VERSION"))).exists()`) and returns immediately on the common/packaged-build case — one `stat` call, no `read_dir`. The `read_dir` scan this item originally described is a **fallback path only**, reached solely when the exact versioned filename doesn't exist (dev-mode naming variants, backwards-compat with the old `agentmux-cef-X.Y.Z` name). The original audit overstated this ("every launch does a directory scan") — corrected here rather than shipping a redundant "fix" for code that already does the fast thing. **Decision (2026-07-03): no code change — audit finding was inaccurate.**
3. ✅ **Feed `StartupEventSink` into the macOS splash** (`unix.rs:204-208` currently drops `rx`) — doesn't itself speed up launch, but is the instrumentation prerequisite for measuring (and then optimizing) macOS phase timings, and is the direct dependency of Prong B. See §B. (#1933)
4. **~~Replace the splash's file-existence busy-poll with a real signal~~ — investigated, deferred.** `splash_mac.rs` polls `std::fs::metadata` every 8ms for up to 10s (`DISMISS_TIMEOUT`) — real, but the item's own description already says it: "minor CPU/wake overhead, not a big latency win." Fixing it properly means replacing the cross-process file-write/poll dismiss protocol (shared between `agentmux-cef`'s `on_load_end` and all three splash backends) with an IPC-socket signal — the same category of "touches a protocol two other working, unverifiable-by-me consumers (Linux's splash, and potentially Windows' event-based equivalent conceptually) rely on" risk that #1931 and #1933 both deliberately avoided for the `"host"`-stage-to-first-paint extension. Low payoff, real cross-crate risk, can't be verified end-to-end here. **Decision (2026-07-03): deferred — not pursued in this implementation stack.** Revisit only alongside the first-paint-signal work already flagged in §B.7/items 1931's and 1933's PR descriptions, since it's the same underlying protocol.
5. **Overlap saga recovery/vacuum with srv spawn** — `unix.rs:213-239` runs both fully before `spawn_srv` begins, with no ordering dependency requiring that. `tokio::join!` instead of sequential `.await`s. Likely small (SQLite ops on a small log are usually sub-10ms) unless the saga log has grown large, but free to parallelize. Applies to Windows/Linux too (shared code).
6. ✅ **CEF `dlopen` cost is real and confirmed to dominate — measured, not hypothesized.** Instrumented and measured on a real launch (2026-07-03, this machine): `dlopen` (CEF framework load) **2960ms**, `cef_init` (`cef::initialize()`) **2613ms** — combined **~5.6s**, the large majority of total launch time. This is the hard-constraint floor (§A.3); the only remaining levers are CEF's own (`--disable-features`, prewarming) or accepting it. See item 7 below for the instrumentation that produced these numbers.
7. ✅ **Host-reported CEF sub-stages, live in the splash panel.** `dlopen` and `cef_init` are now reported by the host (`agentmux-cef`) back to the launcher over the existing IPC connection (new `Command::ReportStartupStageBegin`/`ReportStartupStageEnd`, forwarded directly into `StartupEventSink` bypassing the reducer — same short-circuit pattern as `GetEvents`) and render in the stage panel alongside the launcher's own `saga`/`backend`/`host` stages, on every platform (Windows/Linux already had working consumers; macOS gained one in item 4). `dlopen` can only be reported retroactively (it finishes before `connect_to_launcher` runs); `cef_init` is genuinely live (connect happens before `cef::initialize`). Verified via a real launch on an isolated test channel — not just unit tests.

---

## Prong B — Splash Load-Time Telemetry

### B.1 This already exists — for Windows and Linux

`SPEC_SPLASH_STARTUP_TELEMETRY_2026_06_25.md` (Windows) and `SPEC_SPLASH_TELEMETRY_LINUX_2026_06_27.md` (Linux X11+Wayland) already designed and shipped this feature. **macOS is the only gap.**

| Platform | Live per-phase timing in splash? |
|---|---|
| Windows | Yes — fully implemented (`agentmux-launcher/src/splash.rs`) |
| Linux (X11 + Wayland) | Yes — fully implemented (`splash_linux/mod.rs`, `x11.rs`, `wayland.rs`) |
| macOS | **No** — `splash_mac.rs` never touches `startup_events` |

### B.2 Existing instrumentation (all platforms, already implemented)

`agentmux-launcher/src/startup_events.rs:1-116` defines `StartupEvent::{StageBegin, StageEnd, SubBegin, SubEnd}` with `duration_ms` and `StartupStatus` (Ok/Warn/Error), emitted through a cheap, non-blocking `StartupEventSink` (`mpsc::Sender::send` never blocks). Already-populated stages, cross-platform (shared code paths):

- `"saga"` — `unix.rs:213-239` / `windows.rs:257-282`
- `"migrations"` — `srv_spawner.rs:104,153,156,200-203` (with per-migration `SubBegin`/`SubEnd`)
- `"backend"` (srv spawn/ESTART wait) — `srv_spawner.rs:285,494-519`

**Not yet instrumented on any platform:** a `"host"` stage spanning host spawn → first paint (nothing currently wraps `spawn_host_unix`/CEF-init/first-paint in a stage), and the pre-supervisor path-resolution work in `main.rs:225-322`. Adding both closes end-to-end phase coverage everywhere, not just macOS, and gives Prong A real before/after data once #A.4.1 lands.

### B.3 Why macOS lacks the consumer (confirmed wiring gap, not a design gap)

`main.rs:118,131` always passes `None` for the startup sink on macOS. `unix.rs:199-208`:

```rust
let startup_sink = startup_sink_opt.unwrap_or_else(|| {
    let (s, rx) = startup_events::StartupEventSink::new();
    drop(rx);     // events go nowhere on macOS
    s
});
```

The comment is explicit: *"macOS/other: create a fresh sink and drop rx (macOS splash doesn't yet consume typed events)."* Events are still emitted (saga/migrations/backend fire unconditionally) — they're just discarded. `--splash-selftest`'s macOS branch (`main.rs:208-216`) has the same gap (fires no synthetic events, unlike the Linux/Windows selftest branches at `main.rs:184-220`).

**Why this was deferred, and why it's less work than it sounds:** the splash *must* run on the main thread (AppKit constraint, §A.3), while the supervisor emitting events runs on a worker thread — so wiring this is a cross-thread channel consumed inside the *existing* `run_until_dismissed()` per-tick loop (`splash_mac.rs:352-390`), mechanically the same non-blocking `try_recv()`-per-frame pattern Windows (`splash.rs:282-287`) and Linux (`x11.rs:124-125`, `wayland.rs:127-128`) already use. It is not a new rendering system — it's the same drain-and-apply loop, on a third platform.

### B.4 Design — port the existing pattern to macOS

1. **Thread the receiver through.** `main.rs`'s macOS branch should create `(sink, rx)` itself and pass `Some(sink)` into `launcher_main` instead of hardcoding `None` (mirrors the restructuring already done for Linux at `main.rs:154-162`). `Splash::show()` gains a `startup_rx: Receiver<StartupEvent>` parameter, mirroring `spawn_splash`'s signature (`splash.rs:173-176`) and `splash_linux::spawn`'s (`mod.rs:352`).
2. **Render as `NSTextField` rows, not a custom blitter.** Pre-create N (match Windows' `MAX_STAGE_ROWS = 12`, `splash.rs:74`) label pairs (name + time column) in `build_window()`, next to the existing footer labels (`splash_mac.rs:614-660` is the direct precedent — same font/color conventions apply, see §B.6). Hidden/empty until events arrive. In `run_until_dismissed()`, drain `startup_rx.try_recv()` each tick before the pulse/fade logic and update via `setStringValue:`/`setTextColor:` — cheap, AppKit handles the redraw.
3. **Consolidate the stage-list state machine instead of writing a third copy.** Windows' `StageRow`/`apply_event` (`splash.rs:113-163`) and Linux's `StageList`/`StageEntry` (`splash_linux/mod.rs:107+`) are two independent, near-identical reimplementations already. Move this into a shared module (`startup_events.rs` itself, or a new `splash_stagelist.rs`) and have all three backends consume one implementation — macOS's addition should not be a third copy.
4. **Fixed-height reserved stage area**, matching Windows/Linux's approach (not dynamic window resize) — simplest viable, keeps behavior consistent across platforms. Requires bumping `SPLASH_H` (`splash_mac.rs:52`) by a constant.
5. **Add the summary-hold macOS currently lacks entirely.** Today macOS fades the instant the ready-file appears (`splash_mac.rs:370-375`, no hold at all) — the user never sees a completed timeline. Add the same `AGENTMUX_SPLASH_HOLD_MS`-gated hold Windows uses (`splash.rs:305-313`; default 3000ms, capped ≤1000ms if total < 500ms) before the fade starts.
6. **Wire `--splash-selftest`/`AGENTMUX_SPLASH_DUMP_PNG`** (`main.rs:208-216`, `splash_mac.rs:326-338`) to fire synthetic events like the Linux/Windows selftest branches (`main.rs:182-207`), so the macOS stage panel can be eyeballed via the existing PNG-dump dev affordance without screen-recording permissions.
7. **Close the instrumentation gap from §B.2** — add a `"host"` stage (begin at `spawn_host_unix`, end when the ready-file/IPC-signal from first paint arrives) and a pre-supervisor stage for binary resolution/path work, on all platforms, so macOS's new panel (and Windows/Linux's existing ones) show truly end-to-end phase coverage rather than stopping at "backend."

### B.5 Not meaningfully harder on macOS than it looked

The "structurally different" framing from the Linux telemetry spec (§8 there) is about *which thread renders* and *which text API is used* (`NSTextField`, retained-mode) — not about whether live updates are possible. `setStringValue:` is a legitimate, simpler live-update primitive than Windows' manual DIB compositing. The actual new-code surface is small: steps 1 and 6 above are close to copy-paste of the Linux fix; step 2 reuses the exact footer-label pattern already in the file.

### B.6 Visual/typographic conventions to preserve (already agreed across platforms — do not diverge)

- **Colors:** backdrop `#1A1A1F` (identical constant on Windows `splash.rs:98-100` and macOS `splash_mac.rs:59-61`); footer text `#8A8A93` (same on both). Windows' stage-list palette to reuse: label `#C0C0CC`, completed-time green `#60CC80`, running-time blue `#7090FF`, sub-item gray `#7A7A82`, status ok/warn/err `#44BB44`/`#CCAA33`/`#CC4444` (`splash.rs:83-89`).
- **Footer content is generated once by a single shared function** (`SplashInfo::gather()`/`footer_lines()`, `splash_info.rs:22-42`) specifically so all platforms render identical text (`splash_info.rs:6-7`: *"handed to each platform's splash backend, so the three render identical content"*). Any new stage-list formatting should follow the same one-shared-function principle (per §B.4.3) rather than repeat the current duplication.
- **Font:** Windows/Linux share a baked bitmap monospace font (`splash_font.rs`) via a software glyph blitter (`splash_text.rs`); macOS should keep using its native `NSFont userFixedPitchFontOfSize:13.0` (`splash_mac.rs:633`) rather than importing the bitmap font — visually consistent in spirit (monospace), consistent with "native AppKit" implementation choice.
- **Row format** (carry over verbatim): label left-aligned, truncated past `LABEL_MAX_CHARS = 16` with `..`; time/duration right-aligned; sub-items indented 2 glyph-widths, `SUB_LABEL_MAX_CHARS = 14`; running durations `> 0.4s` while in-flight, replaced by `234ms`/`2.3s` on completion; status glyphs `+`/`!`/`X` for ok/warn/error.
- **Hold duration:** honor the same `AGENTMUX_SPLASH_HOLD_MS` env var name for consistency; confirm the actual shipped Linux default (spec text cites 1500ms design intent — verify against `SUMMARY_HOLD_MS` in `x11.rs`/`wayland.rs` before implementation, don't assume the design doc's number shipped unchanged).

### B.7 Open items to verify before implementation (not confirmed by this pass)

- Whether `cef`/`frontend`-stage events (S5/S6 in the original Windows telemetry spec) or a channel-pruner stage (S3, referencing `SPEC_LOCAL_CHANNEL_PRUNER_2026_06_25.md`) were ever actually wired end-to-end via IPC from `agentmux-cef`/`frontend/app-init.ts` back to the launcher — no evidence found in the launcher-side files inspected. If a future spec/implementation assumes these exist, verify against `agentmux-cef/src/**`, `ipc/server.rs`, and `frontend/app-init.ts` first.
- Exact shipped value of Linux's `SUMMARY_HOLD_MS` (see §B.6).

---

## Non-goals

- Not touching CEF's own internal init sequence, sandboxing model, or subprocess architecture (§A.3) — these are hard constraints.
- Not proposing a Job-Object-equivalent or `PR_SET_PDEATHSIG`-equivalent for macOS in this document — the weak parent-death guarantee is a real gap but is a **reliability** concern, not a **speed** concern, and deserves its own spec if pursued.
- Not redesigning the Windows/Linux splash rendering — both are working reference implementations; macOS should converge toward them, not diverge.
- Not implementing anything yet — this document is the analysis + design; a follow-up implementation pass should scope the two prongs as separate, independently landable changes (A.4.1/A.4.2/A.4.5 are launcher-timing changes with no UI surface; B.4 is the macOS splash consumer).

## Suggested implementation order (for a follow-up pass, not this doc)

1. ✅ Correct CLAUDE.md's stale dev-mode claim (§0) — trivial, unblocks accurate onboarding. (#1930)
2. ✅ `"host"` spawn-latency stage on unix.rs + windows.rs — instrumentation prerequisite, cross-platform benefit. **Scoped down from the original B.4.1/B.4.7 ask**: covers spawn latency only, not full spawn-to-first-paint (see §B.7-style follow-up note added inline at the call sites) — extending to first-paint needs the same race-safe-signal work as item 3 below. The pre-supervisor stage from B.4.7 was deferred alongside it: only Linux currently creates its `StartupEventSink` early enough in `main()` for a pre-supervisor stage to be visible; wiring that up for macOS/Windows too is bundled into item 4 below instead of done twice. (#1931)
3. ~~A.4.1 (parallelize srv/host spawn)~~ — **investigated and deferred, not implemented.** Blocked by a real dependency (srv's dynamic-port self-assignment, reported only via `AGENTMUXSRV-ESTART`) that a simple reorder can't work around — see the struck-through item in §A.4 for the full finding and the two viable-but-bigger fixes. Superseded by item 3′ below as the safe next step.
3′. A.4.5 (overlap saga recovery/vacuum with srv spawn via `tokio::join!`) — smaller but genuinely independent and safe; promoted here from its original slot in item 5.
4. ✅ B.4.1-6 (macOS: thread the receiver, stage-list rendering, hold, selftest wiring) — the visible "load times in splash" deliverable. Delivered as a **self-contained addition** rather than the originally-suggested Windows/Linux consolidation (§B.4.3): `StageRow`/`SubRow`/`apply_event`/formatting live independently in `splash_mac.rs` rather than being extracted into a shared module, since doing that extraction would mean editing two already-working splashes (Windows/Linux) that couldn't be built or run to verify in this environment. The consolidation remains a real, still-open cleanup opportunity — flagged, not done. **The pre-supervisor stage from item 2's deferral was NOT bundled in here as originally suggested** — it's still open; wiring `StartupEventSink` creation earlier in `main()` for macOS/Windows (matching Linux's existing pattern) is deferred to a future pass. (#1933)
5. ✅ A.4.2/A.4.4 (binary-path caching, IPC-signal dismiss) — investigated, **neither implemented**. A.4.2 turned out to already be a non-issue (the fast path was already there; audit finding corrected in §A.4). A.4.4 is real but low-payoff and touches the same cross-crate first-paint-signal protocol that items 2 and 4 both deliberately left alone — deferred alongside them. A.4.5 moved to item 3′ (done there). Pre-supervisor stage (item 2/4's remaining deferral) also still open.

## Stack status (2026-07-03)

All five items in this implementation order have been worked through — three landed as real changes (#1930, #1931, #1932, #1933 — four PRs, since item 2 split into a stage-instrumentation PR and a separate saga/spawn-overlap PR), two investigated and explicitly deferred with reasoning (A.4.1's full srv/host parallelization, A.4.4's IPC-signal dismiss), and one turned out to be a non-issue on closer reading (A.4.2). The two deferred items and the Windows/Linux/macOS `StageRow` consolidation (§B.4.3) are the remaining open follow-ups — each needs its own scoped spec/PR rather than being bundled reactively into this stack.
