# Status & Roadmap — Lifecycle & Crash Architecture Program (as of 2026-07-12)

> **Supersedes** `STATUS_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_07_07.md` (and its 2026-07-11
> addendum) in full. That doc's addendum already noted Pillar 1 Step 4 and Pillar 2 landing;
> this snapshot covers everything since, including one newly discovered, **not yet fixed**
> lifecycle bug found live tonight.

**Type:** Status snapshot + forward roadmap, not a plan doc — a point-in-time picture of the
three-pillar disposability program (`docs/architecture/DISCUSSION_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_06_29.md`)
plus everything adjacent to it that's shipped or been found since 2026-07-07.
**Verify before acting:** re-check file:line references if read more than a few days after
2026-07-12 — this subsystem moves fast (see the historical-audit trend in §5 of the discussion doc:
~45-55% of PRs touch memory/lifecycle/crash).

---

## 0. The one-sentence picture

**All three pillars are functionally complete** (Pillar 1 through Step 5, Step 6 gated on bake
time; Pillars 2 and 3 fully done) and the whole `Client.windowids` leak class is closed — but
tonight's live use surfaced a **new, distinct, not-yet-fixed lifecycle bug**: a native-HWND
label cross-wire that crash-reproject's multi-window creation triggers, causing one window's
title-bar drag to move a *different* window.

---

## 1. Pillar 1 — disposable host

**Goal:** host death (OOM, crash) becomes a visible, bounded reproject instead of a catastrophe.

| Step | What | Status |
|---|---|---|
| 1 | Layout single-writer collapse (`#864`) | ✅ Done, merged. |
| 2 | Persist per-window opacity + floating-pane placement/restore-rect | ✅ Done, merged, live-verified. |
| 3 | Persist window `kind` + parent linkage | ✅ Done, merged, live-verified. |
| 4 | Crash-reproject: fast-path from launcher snapshot, slow-path from srv, restoring-session overlay, splash respawn | ✅ **Done, all 5 phases** (#2014, #2015, #2017, #2032). |
| 5 | E2E test: "host OOM ⇒ session reprojects" | ✅ Done. |
| 6 | Collapse graceful-flush-vs-crash incoherence; shrink saga layer to an in-memory registry | ⬜ **Gated on bake period** — reproject needs ~3-4 weeks of real usage before it's safe to delete the durable-saga fallback it's replacing. Started ~2026-07-11, so roughly early-to-mid August before this can start. |

Reproject's own bake period is *itself* how tonight's new bug (§4) got found — it's exactly the
kind of thing that only surfaces under real multi-window crash/restart cycles, not synthetic
single-window tests.

## 2. Pillar 2 — single lifecycle authority (`reconcile_quit`)

✅ **Done, all 4 phases** (#2080, #2081, #2084, #2083). `reconcile_quit` is the sole quit
decision-maker; the WRR last-user-window path (previously the dominant *unwired* gap) is now the
Draining-gated Stage-2 executor; `orphan_reconcile` is a sanitizer/executor, not a decision-maker.
No open items.

## 3. Pillar 3 — admission control

✅ Shipped independently before this program (#1853). `available_commit_gb()` +
`admit_spawn()` gate refuses agent spawn under commit pressure. Follow-ons (queue-and-drain,
per-agent working-set cap, frontend "memory full" badge) remain open but low-priority and
independent of everything else here.

---

## 4. Lifecycle bugs — found and fixed since 2026-07-07

Bugs adjacent to (not formally inside) the three-pillar program, surfaced by the same
investigative/live-usage pressure the program exists to relieve. Each has its own retro.

| Bug | Status | Retro |
|---|---|---|
| Browser-pane renderer leak (`DestroyWindow` off owner-thread → silent no-op on every pane close) | ✅ Fixed, merged (PR #2000) | `retro-browser-pane-renderer-leak-2026-07-07.md` |
| `Client.windowids` leak, IPC close path | ✅ Fixed (#2087) | — |
| `Client.windowids` leak, registration race | ✅ Fixed (#2088) + `test/e2e/window-close-baseline.e2e.test.ts` | — |
| `Client.windowids` leak, OS-level WM_CLOSE (Alt+F4/taskbar) bypassing IPC entirely | ✅ Fixed (#2089), wndproc subclass hook | — |
| Pool-warmup window `backend_window_id` leak into `Client.windowids` | ✅ Fixed (#2102-era) | — |
| Launcher teardown backstop, UI-thread liveness probe (observe-only) | ✅ Phase 1 merged | — |
| srv crash → host recycle (`SRV_RESTART_BUDGET`) | ✅ Merged (#2107) | — |
| Pagefile/disk: track free space on pagefile volume, warn below threshold | ✅ Merged (#2109) | — |
| **Drag-HWND cross-wire: dragging window N can move a *different* window after a multi-window crash-reproject** | 🆕 **Root-caused tonight, NOT yet fixed.** Task #36. | `retro-reproject-drag-hwnd-crosswire-2026-07-12.md` |

### 4.1 — The new bug, in one paragraph

`capture_hwnd_for_label` (`agentmux-cef/src/commands/window/lifecycle.rs:493`) binds a window's
label to its real Win32 HWND the first time that window's renderer signals "ready" — independently,
per window, with no ordering barrier between windows. When its fast path misses (common in CEF
Views mode), it falls back to "claim whichever of our own visible-but-unclaimed HWNDs `EnumWindows`
returns first" — correct only if windows are created one at a time, an assumption its own comment
states outright. Crash-reproject creates multiple windows back-to-back from a single driver loop,
so two windows' fallbacks can race and cross-wire which label owns which HWND. Once cached, it's
validated (`IsWindow`) and **latches permanently** — not a one-off glitch, a durable mis-binding
for the rest of that session. **This is orthogonal to everything shipped this session**: reproject's
`label_remap` machinery (Step 4) guarantees the *backend/srv* identity of a recreated window is
correct; it never touches the *native HWND* binding, which is where this bug actually lives. If
anything, Step 4 is what makes the bug easy to hit — it's the one mechanism that creates multiple
windows in rapid succession, the precondition the vulnerable fallback assumed away.

Fix direction (not yet implemented, scoped as task #36): thread the actual HWND through from
`CreateWindowTask`'s own `CreateWindowExW` return value at creation time (the UI thread already
knows exactly which HWND belongs to which label the moment it creates it) instead of reconstructing
the mapping later via EnumWindows; or, failing that, serialize the fallback so two windows'
claim-a-HWND sections can never interleave.

---

## 5. Roadmap — open items, ranked

1. **Task #36 — fix the drag-HWND cross-wire (§4.1).** Newest, live-reported, has a confirmed
   root cause and a scoped fix direction. Reasonable next pick given it's a direct, reproducible
   user-facing correctness bug (not a resource leak) and reproject's ongoing bake period will keep
   generating exactly the multi-window-reproject conditions that trigger it.
2. **Task #32 — teardown backstop Phase 2** (armed J0 state machine). Gated on the Phase 1
   liveness probe accumulating a few more days of bake time; probe merged 2026-07-11, so likely
   ready to resume within the next few days.
3. **Task #33 — pagefile P0 item 3**: re-derive the commit-aware scheduler reserve from
   `PrivateUsage` instead of `VirtualMemorySize64`. Last open P0 from `SPEC_WIN10_PAGEFILE_OOM_CRASH`.
4. **Task #34 / #35 — pagefile P1s**: throttle the SetMeta log firehose + rotate/cap
   `agentmux-launcher.log`; prompt (opt-in, never automatic) to shut down an old AgentMux version
   on upgrade. Lower urgency than the P0s.
5. **Task #15 — Residual C**: non-Windows close-path verification. Blocked on macOS/Linux
   hardware access, not on any decision here.
6. **Task #4 — `SPEC_AGENT_SYSTEM_MANAGEMENT_API`**: still awaiting a go/no-go call: not a
   lifecycle-program item, listed here only because it's the other standing open task.
7. **Pillar 1 Step 6**: not startable until the bake period completes (~early-to-mid August 2026),
   tracked for awareness, not action.

---

## 6. Sources

- `docs/status/STATUS_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_07_07.md` (superseded by this doc)
- `docs/architecture/DISCUSSION_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_06_29.md` (program index)
- `docs/specs/SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md`, `SPEC_PILLAR1_STEP4` phases
- `docs/specs/SPEC_PILLAR2_SANITIZE_THEN_DECIDE_2026_07_11.md`
- `docs/retro/retro-browser-pane-renderer-leak-2026-07-07.md`
- `docs/retro/retro-reproject-drag-hwnd-crosswire-2026-07-12.md` (new)
- `docs/specs/SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md`
