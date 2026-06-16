# Memory-Pressure Supervision & Graceful Degradation (host / instance level)

**Status:** proposed — no PR yet
**Date:** 2026-06-16
**Author:** AgentA
**Motivating incident:** `docs/retro/retro-oom-crash-2026-06-16.md`
**Complements (does not replace):** `SPEC_GATED_RENDERER_RECOVERY_2026_06_01.md`

---

## 1. Scope — and how this differs from the renderer spec

AgentMux already has a **renderer-level** memory-aware recovery design
(`SPEC_GATED_RENDERER_RECOVERY_2026_06_01.md`, Phase 1a shipped in PR #1229):
when a *renderer subprocess* OOM-terminates, the host discriminates
"OOM-under-pressure" from "wedged renderer", pauses instead of crash-looping, and
auto-resumes when commit frees. That spec is correct and stays authoritative for
**everything below the browser process**.

But that spec **explicitly scopes out** (its §9) the two layers the 2026-06-16
crash exposed:

> *"The host-level crash-budget relaunch (`spawn_host_supervised`,
> `HOST_RESTART_BUDGET`) and the GPU retry ladder … — unchanged (different
> layer)."*
> *"Cross-process global coordination between AgentMux instances (e.g. a shared
> total-memory budget across instances) — out of scope."*

This spec owns exactly those layers:

| Layer | Owner |
|---|---|
| A renderer subprocess OOMs; the browser process survives | **`SPEC_GATED_RENDERER_RECOVERY`** (renderer pause/resume) |
| The **browser/host process itself** OOMs → whole window/instance dies → launcher relaunches | **THIS SPEC** (memory-aware host supervision) |
| **Proactive, instance-wide** shedding before *any* process OOMs | **THIS SPEC** (CDP purge + Job-Object soft limit) — extends the renderer spec's §6.D |
| **Multi-instance / system** commit exhaustion | **THIS SPEC** (system signal + cross-instance awareness) |
| Graceful **give-up** when relaunch can't recover | **THIS SPEC** (host-native "session saved, reopen to restore") |

One sentence: *the renderer spec keeps a renderer crash invisible; this spec keeps
a **host** crash recoverable and, better, keeps memory pressure from reaching the
cliff in the first place — across the whole instance and with awareness that other
instances exist.*

---

## 2. The incident (summary; full retro linked above)

On 2026-06-16 07:33:16 the v0.44.1 portable's **CEF host process** (PID 3796)
raised Chromium's intentional OOM exception **`0xe0000008`**
(`base::win::kOomExceptionCode`, via `KERNELBASE!RaiseException`) under **system
commit-limit exhaustion** (`errno 1455` / `ERROR_COMMITMENT_LIMIT`). Driver: three
AgentMux instances + a 1.3 GB Vite dev server + repeated builds saturating the
commit pool. The launcher's relaunch ladder (`HOST_RESTART_BUDGET = 3` / 60 s)
relaunched **into the same starved condition** — memory-blind — and the user's
session vanished with no signal.

`0xe0000008` is reported on the **process that lost the allocation race**, which
under CEF's shared exe name can be the *browser* process (whole instance dies) or
a *renderer* (renderer spec's domain). This spec hardens the case the renderer
spec leaves open and reduces how often *either* fires.

---

## 3. Best-practices grounding (researched 2026-06)

**Detection (Windows).**
- `GlobalMemoryStatusEx` gives commit usage by polling — **already sampled** by
  `agentmux-cef/src/memory_heartbeat.rs` (every 20 s) and exposed as the
  `COMMIT_FREE_MB` atomic + synchronous `commit_free_mb()` probe.
- `CreateMemoryResourceNotification(LowMemoryResourceNotification)` +
  `QueryMemoryResourceNotification` give an **event-driven, system-wide** "memory
  is low" signal (non-blocking; reflects the *system*, ignores job limits).
- **Job Object notification limits** (`JobObjectNotificationLimitInformation` /
  `…2` via `SetInformationJobObject`) deliver a **soft**
  `JOB_OBJECT_MSG_NOTIFICATION_LIMIT` to the **job creator's** I/O completion
  port when the job crosses a memory threshold — *without killing anything*.
  Caveat (Old New Thing, 2025-12-29): a process **cannot** get a handle to its
  own enclosing job, so only the *creator* can monitor it. **AgentMux's launcher
  creates Job Object J0** (it owns the handle), so the launcher is the one process
  legitimately able to watch the whole instance's footprint. Hard limits
  (`JOBOBJECT_EXTENDED_LIMIT_INFORMATION.ProcessMemoryLimit` / `JobMemoryLimit`)
  *terminate* on breach — too blunt; we prefer the soft notification.

**Relief without killing (Chromium/CEF).**
- `base::MemoryPressureListener::SimulatePressureNotification(level)` broadcasts a
  MODERATE/CRITICAL pressure signal; V8, Blink, and discardable-memory caches
  respond by purging. Exposed over the **Chrome DevTools Protocol** as
  `Memory.simulatePressureNotification({level:"critical"})` and
  `Memory.forciblyPurgeJavaScriptMemory()` (purges V8 / simulates OomIntervention).
  AgentMux **already runs a `remote_debugging_port`** (`CefSettings` in
  `agentmux-cef/src/main.rs:651-696`), so the host can drive these against its own
  renderers — instance-wide shedding **without** discarding any renderer.

**Proactive eviction (precedent).** Electron's `render-process-gone` reasons
include **`memory-eviction`** — "process *proactively* terminated to prevent a
future OOM" — alongside `oom`. That is precisely the posture here: act *before*
the cliff. (Two cited caveats we must respect: OOM is sometimes misreported as
`crashed` not `oom` — classify defensively; and `--js-flags=--max-old-space-size`
bounds only the V8 old space, **not** total renderer commit — a heap cap is
necessary-not-sufficient.)

Sources are listed in §13.

---

## 4. Design principles (inherited)

From `SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md`: the supervisor is
**passive, bounded, free-rides on persistence that already happens, and is loud
about every action.** We **cannot allocate our way out of system OOM** — so the
levers are only: (a) *delay* relaunch until the OS has commit, (b) *shed our own*
committed memory, (c) *tell the user* honestly, (d) *never lose durable state*
(it already lives in srv's SQLite; `persist_subscriber.rs` writes on every change,
so committed workspace state is already crash-durable).

---

## 5. Design

### 5.A — One memory signal, readable by the launcher (foundation)

`memory_heartbeat.rs` already computes commit-free and publishes `COMMIT_FREE_MB`
**within the host process**. The launcher is a *separate* process and cannot read
that atomic, yet the launcher is where host relaunch is decided. So:

1. **Launcher-side probe.** Add a tiny `commit_free_mb()` to the launcher
   (one `GlobalMemoryStatusEx` call; `/proc/meminfo` on Linux), mirroring the
   host helper. The launcher needs no shared memory — the OS commit figure is
   global. This is the gate for §5.B.
2. **Keep one definition of the thresholds** (`RESUME_FLOOR`, `WARN_FLOOR`, …) in
   `agentmux-common` so host (renderer spec) and launcher (this spec) agree.

> No new measurement framework — both layers read the same OS counter the
> heartbeat already reads.

### 5.B — Memory-aware host relaunch (the core P0 fix)

Today (`agentmux-launcher/src/main.rs`, Windows `run_windows` ~1492-1565, Unix
`run_unix` ~945-1013): on abnormal host exit, relaunch immediately, up to
`HOST_RESTART_BUDGET = 3` within `HOST_RESTART_WINDOW = 60 s`; second try steps to
`--disable-gpu`; budget exhausted → give up. **This is memory-blind.** Changes:

1. **Classify OOM-class host exits.** Treat exit code `0xe0000008`
   (`kOomExceptionCode`, surfaced as `STATUS`-style `3758096392` / `i32 -536870904`)
   — and, defensively, the generic abnormal exit *while* `commit_free_mb()` was
   below `RESUME_FLOOR` at exit time — as **`HostExit::SystemOom`**, distinct from
   `HostExit::Abnormal` (a genuine host bug). (Mirrors the renderer spec's §5
   discrimination, one layer up. Honors the "oom-misreported-as-crashed" caveat by
   falling back to the commit reading.)
2. **Commit-gated, backed-off relaunch for `SystemOom`.** Before relaunching an
   OOM-class exit, probe `commit_free_mb()`. If below `RESUME_FLOOR`, **wait**
   (exponential backoff `RELAUNCH_BACKOFF`: 2 s → 4 s → 8 s → … cap 30 s),
   re-probing, until commit recovers *or* a wall-clock ceiling
   (`OOM_RELAUNCH_DEADLINE`, e.g. 5 min) elapses. Relaunching into a starved
   system just re-OOMs and burns the budget; waiting is the only thing that works.
   The wait must stay **interruptible** — it races against the supervisor's
   shutdown signals (SIGINT/SIGTERM) and srv-death, so a Ctrl+C / quit / backend
   crash during the wait is serviced promptly rather than blocked for up to
   `OOM_RELAUNCH_DEADLINE` (a `select!`-arm-blocking regression reagent caught on
   the P0 PR).
3. **Separate OOM restart budget.** OOM-class relaunches draw from
   `OOM_RESTART_BUDGET` (a larger/longer window — e.g. 5 within 10 min), **not**
   the wedged-host `HOST_RESTART_BUDGET`. A deterministic *host bug* still trips
   the small fast budget; transient *system* OOM does not. (Exactly the
   renderer-spec discrimination — don't let the OS's problem look like our bug.)
4. **`--disable-gpu` earlier for OOM.** The GPU process is a large commit consumer;
   for `SystemOom` relaunches, step to `--disable-gpu` on the **first** retry
   (skip the rung-1 GPU attempt that will likely re-OOM).

### 5.C — Graceful give-up (no more silent vanish)

When relaunch genuinely cannot recover (`OOM_RELAUNCH_DEADLINE` hit, or
`OOM_RESTART_BUDGET` exhausted), the launcher must **not** just `process::exit`.
It paints a **host-native, renderer-free dialog** — reusing the layered-window
machinery `agentmux-launcher/src/splash.rs` already owns (the same fallback the
renderer spec's §6.C uses for its overlay) — saying:

> *"AgentMux ran low on memory and couldn't recover this window. Your panes,
> agents and sign-ins are saved. [Reopen] [Quit]"*

"Reopen" re-execs the launcher once commit is back (it can gate on
`commit_free_mb()` too). Durable state is intact in `objects.db`, so reopening
restores the workspace — the loss is the live window, not the work. This converts
the worst-case (silent disappearance) into an honest, recoverable prompt.

### 5.D — Proactive, instance-wide shedding (prevention; extends renderer §6.D)

The renderer spec's §6.D sheds by **discarding** background browser-pane / hidden-
window renderers (frees commit, recreate on focus). This spec adds the
**non-destructive, instance-wide** lever and the **whole-instance trigger**:

1. **CDP purge.** When `commit_free_mb()` crosses `WARN_FLOOR`, the host drives its
   own `remote_debugging_port`: `Memory.simulatePressureNotification("critical")`
   then `Memory.forciblyPurgeJavaScriptMemory()` across live targets. V8/Blink/
   discardable caches free memory **without** discarding any renderer (no flicker,
   no reload). Cheap, reversible, first thing to try.
2. **Drain the warm pool.** `agentmux-cef/src/commands/window_pool.rs`
   (`POOL_TARGET_SIZE = 2`, ~50-150 MB each) holds spare renderers purely for
   tear-off latency. Under `WARN_FLOOR`, **drain to 0** and suspend refill
   (`spawn_pool_window` already has a quit-state skip guard to reuse); restore the
   pool when commit recovers above `WARN_FLOOR + hysteresis`. This is pure upside:
   pool windows are invisible and reconstructible.
3. **Pause idle agent subprocesses.** An idle agent CLI between turns holds commit.
   Under pressure, the srv can `SIGSTOP`/suspend (Windows: `NtSuspendProcess` /
   suspend threads) idle agent subprocesses and resume on the next turn. (Gated;
   never suspend a streaming turn. Detail TBD — see §9.)

### 5.E — Whole-instance soft ceiling via Job Object J0 (prevention trigger)

The launcher owns Job Object J0 around the whole instance. Set
`JobObjectNotificationLimitInformation(2)` with a **soft** commit/working-set
threshold so the launcher receives a `JOB_OBJECT_MSG_NOTIFICATION_LIMIT` on its
completion port when *this instance's aggregate* footprint crosses a budget — a
per-instance early warning that no per-process probe gives. On that notification
the launcher asks the host (over the existing host pipe) to run §5.D shedding and
shows the §5.F banner. (Soft, not hard — we degrade, we don't let the OS kill.)
This is the one mechanism that bounds a *single instance's* greed; combined with
§5.F it also makes the multi-instance case visible.

### 5.F — User-facing signals (honest, non-modal)

Reuse the existing notification infra — `frontend/app/notification/`
(`usenotification.tsx`, toast bubbles) and `frontend/app/errors/ErrorBanner.tsx`,
driven from srv via the wave pub/sub (`agentmux-srv/src/backend/wps.rs`):

- At `WARN_FLOOR` / on a Job-Object soft-limit hit: a **non-modal** banner —
  *"System memory is low — AgentMux freed caches and paused idle agents. Closing
  some windows or other apps will help."* (Actionable, dismissible.)
- The §5.C give-up dialog is the only modal, and only when recovery failed.
- Surface **how many AgentMux instances are live** in the warning when >1 (see
  §5.G) so the user knows the real lever is "close another instance."

### 5.G — Cross-instance awareness (visibility, not control)

A shared *enforced* budget across instances is genuinely hard (no central
coordinator; isolation invariants I1-I6 forbid cross-instance lifecycle handles).
So scope this to **awareness**:

- A `muxlog`/doctor command reports **current commit-free + count of live
  AgentMux instances** (enumerate by the per-instance pipe / data dir), so pressure
  and its cause are visible *before* a crash.
- The §5.F banner names the instance count. The honest message under multi-instance
  pressure is "you're running N AgentMuxes; close one," and we should say so.
- *Optional, deferred:* an advisory shared counter (a named file/section under
  `~/.agentmux/`) where each instance writes its commit footprint, so each can
  back off proactively when the *aggregate* is high. Advisory only — it must never
  become a cross-instance lifecycle dependency (I2/I3/I4). Likely a later phase.

### 5.H — Cap Chromium's appetite (defense in depth)

In `agentmux-cef/src/app.rs::on_before_command_line_processing` (536-729) add,
behind tunables:
- `--js-flags=--max-old-space-size=<N>` per-renderer V8 old-space cap (necessary
  but **not** sufficient — see §3 caveat; pair with §5.D).
- `--renderer-process-limit=<N>` to bound renderer process count (fewer, shared
  renderers under pressure).
- Consider launching `--disable-gpu` automatically when `commit_free_mb()` is
  already below `WARN_FLOOR` at startup (skip straight to the degraded rung).

The goal: make a runaway hit a **Chromium-level** cap (recoverable: one renderer)
before the **system** commit limit (unrecoverable: a process abort).

---

## 6. Thresholds & constants (tunable; shared in `agentmux-common`)

| Constant | Initial | Meaning |
|---|---|---|
| `RESUME_FLOOR` | 512 MB commit-free | Min headroom to (re)launch a process without instant re-OOM (shared w/ renderer spec) |
| `WARN_FLOOR` | 1 GB commit-free | Trigger §5.D shedding + §5.F banner |
| `RELAUNCH_BACKOFF` | 2→4→8…cap 30 s | OOM-class host relaunch backoff while commit < `RESUME_FLOOR` |
| `OOM_RELAUNCH_DEADLINE` | 5 min | Give up (→ §5.C dialog) if commit never recovers |
| `OOM_RESTART_BUDGET` / window | 5 / 10 min | OOM-class relaunch budget (separate from `HOST_RESTART_BUDGET` 3/60 s) |
| `POOL_DRAIN_BELOW` | `WARN_FLOOR` | Drain warm pool below this; refill above `WARN_FLOOR + 256 MB` (hysteresis) |
| `JOB_SOFT_LIMIT` | e.g. 4 GB commit | Per-instance soft notification limit on J0 (tunable per machine class) |
| `HOST_RESTART_BUDGET` / window | 3 / 60 s (**unchanged**) | Wedged-**host** backstop, still applies to non-OOM abnormal exits |

All commit-free numbers must be validated against real machines (§10); the table
is a starting point, not a measurement.

---

## 7. Host-supervisor state machine (launcher; new arm)

```
        ┌──────────┐  host healthy
        │  Running │◀───────────────────────────────────────────┐
        └────┬─────┘                                             │
   abnormal  │                                                   │ relaunch ok
   exit      ▼                                                   │
     ┌───────────────┐  code==0xe0000008 OR (abnormal &          │
     │  classify     │  commit_free < RESUME_FLOOR at exit)      │
     └──┬─────────┬──┘                                           │
        │SystemOom│ else Abnormal                                │
        ▼         ▼                                              │
 ┌────────────┐ ┌──────────────────────┐                        │
 │ commit-gate│ │ existing path:       │                        │
 │ + backoff  │ │ HOST_RESTART_BUDGET  │── over budget ──▶ give-up (host-native)
 │ (OOM budget│ │ 3/60s, --disable-gpu │                  §5.C dialog
 │  5/10min)  │ └──────────────────────┘                        │
 └──┬──────┬──┘                                                  │
    │commit│ deadline / OOM budget exhausted                    │
    │≥floor│ ───────────────────────────────▶ give-up (§5.C)    │
    ▼      ▼                                                     │
   relaunch (--disable-gpu) ─────────────────────────────────────┘
```

`Running → Abnormal → existing budget path` is **unchanged** for genuine host
bugs. `SystemOom` is the **new** commit-gated arm that waits out the OS instead of
hammering it, and ends in an honest dialog rather than a silent exit.

---

## 8. Phasing

- **P0 — memory-aware host relaunch + graceful give-up (§5.A, §5.B, §5.C).**
  Highest value, smallest surface: the launcher gains a `commit_free_mb()` probe,
  OOM-class exit classification, commit-gated backed-off relaunch on a separate
  OOM budget, and a host-native give-up dialog. Directly converts the 2026-06-16
  failure (memory-blind relaunch → silent vanish) into (wait-then-recover → or
  honest, restorable prompt). No frontend or CEF changes required.
- **P1 — proactive shedding + banner (§5.D CDP purge + warm-pool drain, §5.F
  banner).** Reduces how often P0 fires. CDP purge and pool-drain are
  non-destructive and ship first; idle-agent suspension follows once the
  streaming-turn guard is designed.
- **P2 — instance soft ceiling + system signal + awareness (§5.E Job-Object
  notification limit, system `CreateMemoryResourceNotification`, §5.G doctor/banner
  instance count).** Per-instance budget + multi-instance visibility.
- **P3 — Chromium caps (§5.H)** and the optional advisory cross-instance counter.

Each phase is independently valuable and shippable; P0 alone closes the incident.

---

## 9. Open questions

1. **Idle-agent suspension (§5.D.3)** — suspending a CLI subprocess mid-stream
   would corrupt a turn. Need a precise "agent is idle between turns" signal from
   the block controller, and a resume-before-next-turn guarantee. Defer until that
   signal is specced.
2. **`JOB_SOFT_LIMIT` default** — should it be a fraction of physical RAM, of the
   commit limit, or fixed? Likely `min(4 GB, 0.25 × physical)` — validate.
3. **Reopen UX (§5.C)** — does "Reopen" re-exec the same labeled portable, and how
   does it interact with the single-instance pipe if a sibling is up? Probably
   "Reopen" should just relaunch *this* instance's data dir.
4. **OOM exit-code portability** — `0xe0000008` is Chromium/Windows-specific;
   Linux/macOS OOM surfaces differently (SIGKILL from the kernel OOM killer / Mach
   `EXC_RESOURCE`). The commit-reading fallback (§5.B.1) covers those, but confirm
   the Linux path classifies a kernel-OOM-killed host as `SystemOom`.

---

## 10. Testing

- **Unit (launcher):** table-drive `(exit_code, commit_free_at_exit)` →
  `{SystemOom | Abnormal}`; assert `SystemOom` does **not** consume
  `HOST_RESTART_BUDGET`; assert backoff re-probes and only relaunches above
  `RESUME_FLOOR`; assert `OOM_RELAUNCH_DEADLINE` / `OOM_RESTART_BUDGET` route to
  the give-up dialog.
- **Fault injection:** a debug switch making the host exit with `0xe0000008`, and a
  stubbed `commit_free_mb()` provider, to exercise the gated-relaunch loop without
  real pressure.
- **Soak (the real condition):** shrink the page file, launch 2-3 instances + a
  build to exhaust commit, confirm: (a) the host **waits** instead of crash-looping
  relaunch, (b) warm pool drains + CDP purge fires at `WARN_FLOOR`, (c) the banner
  names the instance count, (d) on continued exhaustion the give-up dialog appears
  (not a silent exit) and "Reopen" restores the workspace from `objects.db`.
- **Anti-flap:** oscillate commit around `RESUME_FLOOR`; assert no relaunch thrash
  and no OOM-budget exhaustion from oscillation alone.
- **Regression:** a non-OOM abnormal host exit (commit healthy) still uses the
  unchanged `HOST_RESTART_BUDGET` fast path and trips give-up after 3/60 s.

---

## 11. Out of scope / unchanged

- **Renderer-subprocess OOM pause/resume** — owned by
  `SPEC_GATED_RENDERER_RECOVERY_2026_06_01.md` (§6.B/§6.C). This spec does not
  touch `on_render_process_terminated`.
- **Raising the OS commit limit** — system-managed; impossible from a process.
- **Freeing *other* applications' memory** — impossible.
- **Enforced cross-instance memory budget** — out (only advisory awareness, §5.G);
  enforcement would violate isolation invariants I2/I3/I4.
- **The renderer crash budget, recovery page, and draft persistence** (renderer
  spec §6.E) — unchanged.

---

## 12. Risks

- **Backoff too conservative → window stays down longer than needed.** Mitigated
  by short initial backoff (2 s) + immediate relaunch the instant commit clears
  `RESUME_FLOOR`; the deadline only bounds the pathological never-recovers case.
- **Job-Object notification false-positives** on machines with huge page files
  (commit "free" but RAM thrashing). The Job-Object limit is per-instance commit,
  not system RAM; tune `JOB_SOFT_LIMIT` and treat it as advisory (drives shedding,
  not kills).
- **CDP purge cost** — `forciblyPurgeJavaScriptMemory` triggers GC pauses; gate it
  to `WARN_FLOOR` crossings (not steady-state) and rate-limit.
- **`--max-old-space-size` false security** — it does **not** bound total renderer
  commit (§3 caveat #2); never rely on it alone — it pairs with §5.D shedding.
- **Give-up dialog under true exhaustion** — must be renderer-free (layered-window,
  reuse `splash.rs`) precisely because a renderer may not start; same constraint
  the renderer spec's §6.C overlay already solved.

---

## 13. Cross-references & sources

**Internal (do not duplicate):**
- `docs/retro/retro-oom-crash-2026-06-16.md` — the motivating incident.
- `SPEC_GATED_RENDERER_RECOVERY_2026_06_01.md` — the renderer-level layer this
  complements (renderer pause/resume; §6.D shedding this extends instance-wide;
  §6.C native overlay this reuses for give-up).
- `SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md` — the stability mandate +
  host-level supervisor + `HOST_RESTART_BUDGET` ladder this makes memory-aware.
- `SPEC_GRACEFUL_CRASH_HANDLING_2026_04_13.md` — the renderer recovery page.
- `SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md` — invariants I1-I6 that
  constrain §5.G cross-instance work.
- `docs/MEMORY_HEARTBEAT_SPEC.md` — the heartbeat + `commit_free_mb()` this reads.

**Integration points (from codebase survey):**
- Launcher supervisor: `agentmux-launcher/src/main.rs` (Windows ~1082-1581, Unix
  ~689-1070; budget `283-284`; `--disable-gpu` in `spawn_host_supervised` 294-374).
- Memory signal: `agentmux-cef/src/memory_heartbeat.rs` (`COMMIT_FREE_MB` 15-41,
  `GlobalMemoryStatusEx` 109-177).
- CEF flags: `agentmux-cef/src/app.rs::on_before_command_line_processing` 536-729;
  `CefSettings`/`remote_debugging_port` `agentmux-cef/src/main.rs` 651-696.
- Wave pub/sub: `agentmux-srv/src/backend/wps.rs`. Notifications:
  `frontend/app/notification/`, `frontend/app/errors/ErrorBanner.tsx`.
- Persistence: `agentmux-srv/src/persist_subscriber.rs` (on-change SQLite writes),
  `objects.db` (`agentmux-srv/src/main.rs:375`).
- Warm pool: `agentmux-cef/src/commands/window_pool.rs` (`POOL_TARGET_SIZE` 60,
  `spawn_pool_window` 86-150).
- srv crash dumps: `agentmux-srv/src/crash_monitor.rs` (no host equivalent → WER).

**External best-practices (researched 2026-06):**
- Windows memory detection (system + job): *The Old New Thing*, "How can I detect
  that the system is running low on memory? Or that my job is running low on
  memory?" (2025-12-29).
- `QueryMemoryResourceNotification` / `CreateMemoryResourceNotification` /
  `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` — Microsoft Learn (Win32 memoryapi/winnt).
- Chrome DevTools Protocol **Memory** domain — `simulatePressureNotification`,
  `forciblyPurgeJavaScriptMemory`, `setPressureNotificationsSuppressed`
  (chromedevtools.github.io).
- Chromium `base::MemoryPressureListener::SimulatePressureNotification`
  (chromium.googlesource.com base/memory).
- Electron `render-process-gone` reasons incl. `oom` / `memory-eviction`
  (electronjs.org RenderProcessGoneDetails) + heap-cap caveat (electron#37214) +
  oom-misreported-as-crashed caveat (electron#40426).
```
