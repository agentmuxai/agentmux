# Gated Renderer Recovery — Memory-Aware Crash Handling

**Status:** Phase 1a implemented (PR #1229); Phases 1b/2/3 designed
**Date:** 2026-06-01
**Author:** AgentA
**Tracking:** open — no PR yet

---

## 1. The incident

On 2026-06-01 a 0.41.1 host running alongside several other AgentMux instances
hit a Chromium renderer **OOM** termination. The host's own crash log:

```
ERROR target=crash kind="renderer_terminated" reason="out of memory" detail="Out of Memory"
```

The memory heartbeat (`agentmux-cef/src/memory_heartbeat.rs`) showed the cause
held for **~20 minutes** before the crash:

```
avail_phys_gb=18.0     ← physical RAM fine
avail_page_gb=0.0      ← system COMMIT LIMIT (RAM + page file) exhausted
```

Physical RAM was never the problem. The Windows **system commit limit** (87.7 GB
of RAM + page file) was 100% committed by the aggregate of many simultaneously
running AgentMux instances × their full Chromium process trees (GPU, network,
storage, N renderers). When commit is pinned at zero, the next allocation in any
renderer fails and Chromium OOM-terminates it. The host *main* process survived
throughout (steady ~117 MB, no `LOG(FATAL)`, no exit) — only a renderer
subprocess died.

This is **not an application defect**. It is the OS being out of commit, which no
process can allocate its way out of. The question this spec answers: *given that
the fault is unavoidable, how do we make recovery graceful instead of a manual,
crash-looping, sometimes-blank-screen experience?*

---

## 2. Current behavior (what already exists)

`on_render_process_terminated` in `agentmux-cef/src/client/mod.rs:1338`
(per `SPEC_GRACEFUL_CRASH_HANDLING_2026_04_13.md`) already does a lot right:

1. **Classifies** the termination: `PROCESS_OOM` → "out of memory",
   `PROCESS_CRASHED`, `ABNORMAL_TERMINATION`.
2. **Rate-limits** the `target=crash` log (≤1 line / 100 ms; suppressed count
   carried forward) — from the 2026-05-28 884 MB log-spam retro.
3. **Per-browser crash budget**: `CRASH_BUDGET = 3` crashes within
   `CRASH_BUDGET_WINDOW = 10s` (`mod.rs:11,17`). Over budget → a terminal
   **"give up" page** (`crash_loop_terminal_page`) that does NOT reload — breaks
   the wedged-slot infinite loop.
4. **Under budget** → loads an HTML **recovery page** with **Reload / Quit**
   buttons. Reload navigates to the real app URL, which spawns a fresh renderer
   and re-projects all state from the sidecar.

State durability is already a free-ride: the renderer is **state-poor**. All
durable state (workspaces, tabs, panes, layouts, agents, conversation history)
lives in **srv's SQLite**, a separate process that survives renderer deaths. A
reloaded renderer re-projects it exactly. So a renderer crash loses nothing
*except* live renderer-only state (see §6.E).

---

## 3. The gap — this design misfires under *sustained* memory exhaustion

The existing design is correct for a **one-off** renderer crash and for a
**genuinely wedged** renderer slot. It misfires for the **sustained
system-OOM** case in four specific ways:

| # | Gap | Consequence |
|---|-----|-------------|
| G1 | The recovery page's **Reload is manual and memory-blind**. The user clicks Reload while commit is still at 0. | The fresh renderer immediately re-OOMs. White → recovery page → Reload → white → … |
| G2 | The **crash budget conflates "OOM under pressure" with "wedged".** Three OOM re-tries in 10s (trivially hit if the user clicks Reload a few times, or if anything auto-reloads) trips the **"give up" page**. | The app declares "unrecoverable, give up" when the truth is "recoverable in ~60 s once memory frees." |
| G3 | The recovery page **itself needs a renderer to render**. Under true commit exhaustion the data-URI recovery page's renderer may fail to start. | White screen, no recovery page at all — the worst case. |
| G4 | Nothing is **memory-aware proactively**. The heartbeat saw `avail_page_gb=0` for 20 min and did nothing — no load-shedding, no pre-warning, no coupling to the recovery decision. Draft composer text is lost. | The crash happens that could have been avoided or softened, and unsent typed text vanishes. |

---

## 4. Design principle

From `SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md` §1/§3 (the stability
mandate): *"No crashes ever" = the user must never SEE a crash. Every fault
becomes an invisible, sub-second auto-recovery — at most a flicker.* The
supervisor is **passive, bounded, free-rides on persistence that already
happens, and is loud about every action.**

Applied here:

- **We cannot allocate our way out of system OOM** — don't try. Ride it out and
  resume. (Honest scope: from a transient spike, full recovery; from sustained
  exhaustion, *graceful pause + auto-resume when the OS allows*, never
  "allocate through it".)
- **Free-ride on what exists**: the memory heartbeat already measures
  `avail_page_gb`; srv already persists all durable state; the splash machinery
  already proves native overlay rendering; `on_render_process_terminated`
  already classifies OOM. This spec wires those together; it does not build a
  new framework.
- **Bounded**: the crash budget stays as the wedged-slot backstop. The memory
  gate is additive, not a replacement.

---

## 5. The core idea

Split the renderer-termination response by **whether the system is memory-starved
right now**, measured from the heartbeat's `avail_page_gb`:

```
renderer terminated
  ├─ status == PROCESS_OOM  AND  avail_page_gb < RESUME_FLOOR
  │     → MEMORY-GATED PAUSE  (this is system pressure, not a broken renderer)
  │       • does NOT consume the crash budget
  │       • show "Paused — low system memory, resuming…" (native overlay if
  │         the renderer can't render the HTML recovery page — §6.C)
  │       • poll avail_page_gb; auto-reload when it recovers above RESUME_FLOOR
  │
  └─ otherwise  (non-OOM crash, OR OOM while memory IS available)
        → EXISTING PATH, unchanged
          • consume crash budget (3 / 10 s)
          • under budget → Reload/Quit recovery page
          • over budget  → "give up" terminal page
```

The key discrimination: **OOM-while-memory-low is the OS's fault and transient —
it must not burn the wedged-slot budget.** OOM-while-memory-available *is* the
renderer's fault (a genuine leak/wedge) and **must** consume the budget. This
single distinction fixes G1 and G2 directly.

---

## 6. Design detail

### 6.A Memory-availability signal

`memory_heartbeat.rs` already computes `avail_page_gb` every 20 s. Two changes:

1. Publish the latest `avail_page_gb` (and `load_pct`) to a process-wide
   `AtomicU64` (millibytes or fixed-point) that the crash handler and the
   load-shedder can read without blocking.
2. On the crash path, the 20 s cadence is too coarse to gate a resume. Add an
   **on-demand probe**: a cheap synchronous `GlobalMemoryStatusEx` call
   (microseconds) the gated-pause loop calls directly when deciding whether to
   resume. The heartbeat stays the background publisher; the probe is the
   fine-grained gate.

`RESUME_FLOOR` — the commit-free threshold above which it's safe to spawn a
renderer. A renderer's initial commit is on the order of 100–200 MB; set
`RESUME_FLOOR` with margin (e.g. **512 MB** commit-free) so the resumed renderer
doesn't instantly re-OOM. Tunable; see §7.

### 6.B OOM-gated pause (the heart of the fix)

When the discrimination in §5 selects the gated-pause arm:

- **Do not** push to `crash_history` (the budget deque) — this is not a
  budget-consuming crash.
- Enter a per-browser `MemoryPaused` state. Show the paused UI (§6.C).
- Start (or join) a single shared **resume watcher**: a host thread that probes
  commit-free on a short interval (e.g. 2 s) with backoff, and when it observes
  `avail_page_gb ≥ RESUME_FLOOR` *sustained for K consecutive probes* (debounce
  against flapping), reloads the paused browser(s) by navigating to the real app
  URL — exactly what the existing Reload button does.
- **Anti-flap guard**: if a browser resumes and then OOMs *again within
  `REOOM_GUARD_WINDOW` while memory was above the floor at resume time*, that
  resume counts as a real crash and consumes the budget — this catches a
  genuinely broken renderer that OOMs even with headroom, so we still converge
  on the "give up" page rather than an infinite memory-gated loop.

### 6.C Paused UI — HTML page first, host-native overlay fallback (fixes G3)

Two render paths, tried in order:

1. **HTML paused page** (preferred): a variant of the existing recovery page
   with no Reload button (resume is automatic) and a "Quit" button, plus
   "Resuming automatically when memory frees." Loaded the same way
   (`frame.load_url(data:…)`). Works when commit has *some* headroom.
2. **Host-native overlay** (fallback): under true commit exhaustion even the
   data-URI page's renderer can fail to start (G3). The host owns the OS window
   and is alive, so it paints a **layered-window overlay** using the exact
   machinery `agentmux-launcher/src/splash.rs` already uses
   (`CreateWindowExW(WS_EX_LAYERED|WS_EX_TOPMOST|WS_EX_NOACTIVATE)` +
   `UpdateLayeredWindow` + premultiplied DIB). No renderer required. The overlay
   shows "Paused — system memory low. Resuming…" and is torn down when the
   resume watcher reloads the renderer.

   Detection of "the HTML page itself failed": if a browser in `MemoryPaused`
   does not reach `on_load_end` for the paused data-URI within a short timeout,
   escalate to the native overlay.

   **Window-renderer vs browser-pane scope**: a window-level renderer OOM blanks
   the whole window → native overlay on that window. A browser-pane child
   renderer OOM is localized → the main window renderer is alive and can render
   the HTML paused state in that pane's slot (no native overlay needed).

### 6.D Proactive load-shedding (fixes part of G4) — Phase 2

Before the OOM, when the published `avail_page_gb` crosses a **warn** threshold
(e.g. < 1 GB commit-free), shed *our own* committed memory — the one lever we
control:

- Discard background **browser-pane** child renderers (separate processes;
  discarding frees real commit; re-create on focus).
- Suspend renderers of **hidden/minimized windows**.

Note: panes *within* one window share that window's single renderer, so
"freeze a background pane" frees no process — the unit of shedding is a
**window renderer** or a **browser-pane renderer**. There is no pane-visibility
atom today (only focus/blur listeners in `termwrap.ts:195`); Phase 2 adds the
minimal visibility signal needed.

Also surface a **non-modal** "system memory critically low — close some windows"
hint at the warn threshold so the user can act before the cliff.

### 6.E Proactive draft preservation (fixes part of G4) — Phase 3

The agent composer textarea (`AgentFooter.tsx`) is **uncontrolled and
in-renderer only** — unsent text is lost on renderer death. A reactive
"snapshot on OOM" is **impossible**: the renderer is already dead when the host
learns of the crash; you cannot run JS in a dead process.

Therefore preservation must be **proactive**: debounced sync of composer draft
text to srv while the user types (e.g. 500 ms idle), stored against the block.
On renderer reload the composer rehydrates the draft from srv. Everything else
already survives via srv; this closes the last in-renderer-only gap, making the
recovery truly zero-loss.

---

## 7. Thresholds & constants (all tunable, single source)

| Constant | Initial | Meaning |
|---|---|---|
| `RESUME_FLOOR` | 512 MB commit-free | Min headroom to spawn a renderer without instant re-OOM |
| `WARN_FLOOR` | 1 GB commit-free | Trigger load-shedding + user hint (Phase 2) |
| `RESUME_PROBE_INTERVAL` | 2 s | Resume-watcher poll cadence (with backoff to ~10 s) |
| `RESUME_DEBOUNCE_K` | 3 probes | Consecutive above-floor probes before resuming (anti-flap) |
| `REOOM_GUARD_WINDOW` | 15 s | Resume-then-OOM within this *with* headroom ⇒ counts as a real crash |
| `DRAFT_SYNC_DEBOUNCE` | 500 ms | Composer→srv draft persistence idle (Phase 3) |
| `CRASH_BUDGET` / `_WINDOW` | 3 / 10 s (unchanged) | Wedged-slot backstop, still applies to non-OOM + headroom-OOM |

---

## 8. State machine (per browser)

```
        ┌──────────┐  renderer OK
        │  Healthy │◀───────────────────────────┐
        └────┬─────┘                             │
   OOM &     │     non-OOM crash, OR             │ resume watcher:
   mem<floor │     OOM & mem≥floor               │ mem≥RESUME_FLOOR
             │     → existing budget path        │ ×K, then load_url(app)
             ▼                                    │
      ┌─────────────┐                             │
      │ MemoryPaused│─────────────────────────────┘
      │ (no budget) │
      └──────┬──────┘
             │ resumed then OOM again WITH headroom (within REOOM_GUARD_WINDOW)
             ▼
      ┌──────────────┐  > CRASH_BUDGET in window
      │ existing path│───────────────────────────▶ "give up" terminal page
      └──────────────┘
```

`Healthy → (existing path) → Reload/Quit recovery page → give-up` is the
**unchanged** legacy flow. `MemoryPaused` is the **new** arm that never reaches
"give up" while the fault is purely system pressure.

---

## 9. What is explicitly unchanged / out of scope

- The non-OOM crash path, the recovery page, and the crash budget for
  wedged slots — **unchanged**.
- The host-level crash-budget relaunch (`spawn_host_supervised`,
  `HOST_RESTART_BUDGET`) and the GPU retry ladder (rung-2 `--disable-gpu`) in
  the launcher — **unchanged** (different layer; this spec is renderer-level).
- We do **not** attempt to raise the OS commit limit (system-managed; it already
  auto-grew) or free other processes' memory (impossible).
- Cross-process global coordination between AgentMux instances (e.g. a shared
  total-memory budget across instances) — **out of scope**; each instance only
  manages its own renderers.

---

## 10. Phasing

- **Phase 1a (the discrimination — SHIPPED)** — §6.A memory signal
  (`commit_free_mb()` + published atomic in `memory_heartbeat.rs`); §6.B the
  OOM-vs-pressure discrimination with crash-budget **bypass** and a separate
  `MEMORY_PAUSE_BUDGET` backstop; §6.C the **manual** recoverable "low memory"
  paused page (Resume → re-project from srv / Quit), memory-guided so the user
  frees memory before resuming. This stops the crash budget from falsely
  declaring "give up" under transient system OOM, and replaces the
  reload→re-crash→give-up experience with an honest, recoverable paused state
  that loses nothing. Compile-verified; runtime OOM smoke pending (see §11).
  **Highest value; shipped first.**
- **Phase 1b (automatic + total-exhaustion)** — host-driven, memory-gated
  **auto-resume** (`post_delayed_task` on the UI thread, probing
  `commit_free_mb()` with `RESUME_DEBOUNCE_K` + backoff, navigating the paused
  browser when commit recovers) so the window comes back *on its own*; plus the
  §6.C **host-native overlay** (reusing `agentmux-launcher/src/splash.rs`'s
  layered-window machinery) for the case where commit is so exhausted that even
  the paused HTML page can't render. Requires a real induced-OOM smoke, so it is
  split from 1a rather than shipped blind.
- **Phase 2 (prevention)** — §6.D proactive load-shedding + low-memory hint.
  Reduces how often Phase 1 is even needed.
- **Phase 3 (zero-loss)** — §6.E proactive draft persistence to srv.

---

## 11. Testing

- **Unit (host)**: the §5 discrimination — table-drive `(status, avail_page_gb)`
  → `{MemoryPaused | budget-path}`; assert `MemoryPaused` does not push to
  `crash_history`; assert resume requires K consecutive above-floor probes;
  assert resume-then-OOM-with-headroom consumes the budget.
- **Fault injection**: a debug command to force `on_render_process_terminated`
  with `PROCESS_OOM` and a stubbed `avail_page_gb` provider, to exercise the
  pause/resume loop without real memory pressure.
- **Manual / soak**: reproduce the real condition — cap the page file small,
  open enough panes to exhaust commit, confirm: (a) panes pause rather than
  crash-loop, (b) the native overlay appears when the HTML page can't render,
  (c) panes auto-resume and re-project full state when memory frees, (d) draft
  text survives (Phase 3).
- **Anti-flap**: oscillate `avail_page_gb` around `RESUME_FLOOR`; assert no
  resume/OOM thrash, no budget exhaustion from the oscillation alone.

---

## 12. Cross-references (do not duplicate)

- `SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md` — the stability mandate,
  supervisor prime directive, host-level retry ladder. This spec is the
  renderer-level, memory-aware refinement of §4's Chromium-children manager.
- `SPEC_GRACEFUL_CRASH_HANDLING_2026_04_13.md` — the existing recovery page +
  crash budget this spec extends.
- `docs/analysis/CRASH_GPU_PROCESS_FATAL_2026_05_20.md` — the GPU-process FATAL
  cascade (6 GPU crashes → host `LOG(FATAL)` → 0x80000003 modal). Same
  memory-exhaustion root; different subprocess. The load-shedding in §6.D also
  reduces GPU-process pressure.
- `docs/retro/retro-portable-rm-running-install-2026-05-28.md` — the 139k-crash
  / 884 MB log-spam incident that motivated the rate-limit + crash budget this
  spec preserves.
- `docs/MEMORY_HEARTBEAT_SPEC.md` — the heartbeat this spec reads from.

---

## 13. Risks

- **Resume too eager** → resumed renderer re-OOMs. Mitigated by `RESUME_FLOOR`
  margin + `RESUME_DEBOUNCE_K`. If it still flaps, the anti-flap guard routes it
  to the existing budget path so it can't loop forever.
- **Native overlay z-order / input** — the overlay must be `WS_EX_NOACTIVATE`
  and not steal focus or block the close button; reuse splash's exact flags.
  Browser-pane case avoids the overlay entirely.
- **Load-shedding visible flicker** (Phase 2) — discarding a background
  browser-pane renderer then re-creating on focus must restore URL + scroll;
  reuse the browser-pane lifecycle (`Closing` state machine) already specced for
  redock.
- **Draft sync write amplification** (Phase 3) — debounce + only persist on
  change; draft is small text. Negligible vs the existing per-keystroke srv
  traffic.
