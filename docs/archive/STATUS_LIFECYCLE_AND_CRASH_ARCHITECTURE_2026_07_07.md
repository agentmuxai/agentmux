# Status — Lifecycle & Crash Architecture Program (as of 2026-07-07)

> **SUPERSEDED (addendum 2026-07-11).** This snapshot predates a week that closed most of what
> it lists as open. Landed since:
>
> - **Pillar 1 Step 4 (crash-reproject) — DONE, all 5 phases** (#2014, #2015, #2017, #2032):
>   two-tier fast/slow reproject, restoring-session overlay, opt-in E2E suite
>   (`test/e2e/crash-reproject.e2e.test.ts`). §1's "Step 4 not started" and everything blocked
>   on it no longer holds. Follow-up hardening: #2048 (startup window storm), #2053 (silent
>   CloseWindow reducer no-op).
> - **Pillar 2 (sanitize-then-decide) — DONE, all 4 phases** (#2080, #2081, #2084, #2083):
>   `reconcile_quit` is the sole quit decision-maker; WRR demoted to the Draining-gated Windows
>   Stage-2 executor; orphan_reconcile is a sanitizer/executor. §2's "one hard gap left" is closed.
>   Spec closure: `SPEC_PILLAR2_SANITIZE_THEN_DECIDE_2026_07_11.md` + §3.3 rows in
>   `SPEC_PILLAR2_WIRE_RECONCILE_QUIT_2026_06_29.md`.
> - **`Client.windowids` leak class — CLOSED, all three entry points** (#2087 IPC close path,
>   #2088 registration race + `test/e2e/window-close-baseline.e2e.test.ts`, #2089 OS-level
>   WM_CLOSE wndproc routing).
>
> Program remaining as of 2026-07-11: crash-reproject bake period (~3–4 weeks of real usage)
> → Step 6 saga collapse; floater placement read-back check. Everything below is the historical
> 2026-07-07 picture, kept verbatim.

**Type:** Status snapshot, not a plan — a point-in-time picture of the three-pillar disposability
program (`docs/architecture/DISCUSSION_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_06_29.md`).
**Verify before acting:** every claim below is checked against source or a live test as of this
date; re-verify file:line references if this doc is read more than a few days after 2026-07-07 —
the surrounding code moves fast.

---

## 0. The one-sentence picture

Of the three pillars, **Pillar 3 is done**, **Pillar 2 is partially wired with one hard, well-scoped
gap left**, and **Pillar 1 has all its prerequisite data-persistence work done but the actual
disposability payoff (crash → automatic rebuild) does not exist yet.**

---

## 1. Pillar 1 — disposable host

**Goal:** host death (OOM, crash) becomes a visible, bounded reproject instead of a catastrophe —
srv is durable truth, host rebuilds from it on any (re)start, not just a clean cold boot.

| Step | What | Status |
|---|---|---|
| 1 | Layout single-writer collapse (`#864`) — `db_layout` has exactly one writer (the reducer), no split-brain | ✅ **Done, merged.** All 5 phases (`SPEC_864_LAYOUT_SINGLE_WRITER_2026_06_30.md`). Capstone invariant test in `agentmux-srv/src/server/tests.rs`. |
| 2 | Persist the two known host-only topology facts: per-window opacity, floating-pane placement/restore-rect | ✅ **Done, merged, live-verified.** `SPEC_PILLAR1_STEP2_WINDOW_TOPOLOGY_PERSISTENCE_2026_07_06.md`, both slices. Read-back-on-reopen for floating panes deliberately deferred to Step 4 (no live trigger until reproject exists). |
| 3 | Persist window `kind` (FullInstance/Subwindow) + parent linkage | ✅ **Done, both phases merged** (#2004, #2007), live-verified. `SPEC_PILLAR1_STEP3_WINDOW_TOPOLOGY_2026_07_07.md`. **Corrected a real gap in the original design doc** — this data had *zero* srv representation before, not just "needs an audit." |
| 4 | Fire the cold-start restore path on crash, covering **all** windows (not just one), with in-flight work re-derived from topology rather than resumed | ⬜ **Not started. Not even fully scoped yet.** See §1.1. |
| 5 | E2E test: "host OOM ⇒ session reprojects" | ⬜ Blocked on Step 4. |
| 6 | Collapse the graceful-flush-vs-crash incoherence; shrink the saga layer to an in-memory registry (nothing durable left to compensate) | ⬜ Blocked on Step 4. |

### 1.1 — Why Step 4 is the real remaining work, and why it's not just "wire an existing path"

A 2026-07-07 research pass (grounding `SPEC_PILLAR1_STEP3`) falsified the original design doc's
headline claim that "the reproject read path already exists — cold start already reads srv topology
and rebuilds it, Pillar 1 just needs to fire that on crash." Verified instead:

- Cold launch (`agentmux-cef/src/app.rs::on_context_initialized`) unconditionally creates **exactly
  one** native window; the frontend inside it reads only `Client.windowids[0]`
  (`frontend/app-init.ts:317-339`). A second FullInstance window or a Subwindow is **never**
  automatically recreated today, on any relaunch — only by explicit user/agent action.
- So Step 4 needs genuinely new code: enumerate `Client.windowids[1..]` (now durable and readable
  thanks to Step 3), resolve each one's `kind`/`parent_window_id`, and drive the equivalent of
  `open_new_window`/`open_subwindow` per entry — a capability that doesn't exist in any form today.
- The in-flight-work re-derivation rule (design doc §7's "subtle part": derive desired end-state
  from topology, don't resume a half-finished create) has **zero existing scaffolding**. The closest
  structural analog, `commands/orphan_reconcile.rs`'s live/dead/hostless planner, solves a different
  trigger (last-window-close reconciliation, not crash-reproject) and isn't directly reusable.
- **Also unresolved:** there is still no code anywhere that distinguishes "this is a fresh launch"
  from "this is a post-crash relaunch" — the launcher's supervisor (`agentmux-launcher/src/
  supervisor/windows.rs`) replays the identical spawn args on every restart. Step 4 needs this
  distinction (or needs to make the same cold-start path idempotently safe to run unconditionally
  every time, which is a design choice, not yet made).

**Bottom line: Step 4 needs its own dedicated design spec before any code, the same way Step 2/3
each got one.** It is a materially bigger and more novel piece of work than Steps 1-3 combined — those
were all "persist a fact host already knows"; Step 4 is "build a reconstruction engine that doesn't
exist in any form."

---

## 2. Pillar 2 — single lifecycle authority (`reconcile_quit`)

**Goal:** "should the app quit now?" has exactly one decision-maker (`reducer::quit::reconcile_quit`,
a pure function of `HostState`), with every other site reduced to a pure executor of that decision —
replacing today's three independent, occasionally-racing quit-decision sites.

| Site | Status |
|---|---|
| `client::on_before_close` | ✅ **Wired** (PR #1993). Consumes `reconcile_quit`'s verdict via `DispatchOutput.request_drain` instead of re-deriving its own count. Live-verified: single-window close, sequential multi-window close, both clean, no deadlock. |
| `wrr::win_event::maybe_quit_on_last_user_window` | ⬜ **Not wired — and this is the dominant path, not an edge case.** Live-verified 2026-07-07 (during the on_before_close work): closing the **main window** on Windows never fires `on_before_close` at all — quit is driven entirely by this OS-level `EnumWindows` hook, which calls `quit_message_loop()` directly, bypassing `reconcile_quit`/`QuitState` completely. Wiring this needs the reducer to learn about an OS-level close signal it currently has no channel for — a real design task (`SPEC_PILLAR2_WIRE_RECONCILE_QUIT_2026_06_29.md` §3.3 flags it, not yet resolved). **🔧 2026-07-08: the false-positive symptom of this gap (closing a non-last window kills the whole host) is now scoped and being implemented** — `SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08.md`, a minimal slice that does not attempt the full wiring described above. |
| `commands::orphan_reconcile` | ⬜ **Not wired.** Its `plan.begin_drain` computation carries a "Race B" (freshly-promoted-HWND) guard with no `HostState` equivalent — merging it into `reconcile_quit` needs either a new `HostState` field or a two-phase "sanitize state.browsers, then trust reconcile_quit" design. Not started. |

**Practical read:** Pillar 2's most-used call site (closing the last window, the common case) still
runs on the pre-Pillar-2 path. The wiring that *has* landed only covers secondary-window and
pool-window closes. This is real progress (net-simplification, zero regressions, live-verified) but
should not be reported as "Pillar 2 is wired" without this caveat.

---

## 3. Pillar 3 — admission control

✅ **Shipped before this session (#1853), independently of Pillars 1/2, as the design doc predicted.**
`sysinfo::available_commit_gb()` + `runner::admit_spawn()` gate refuses agent spawn below
`AGENTMUX_AGENT_COMMIT_RESERVE_GB`. No further work this session. Follow-ons (queue-and-drain instead
of hard refuse, per-agent working-set cap, frontend "memory full" badge) remain open but are
independent of everything else in this doc and can land anytime.

---

## 4. A related, adjacent finding: the browser-pane renderer leak (fixed)

Not part of the three-pillar program directly, but surfaced by the same investigative pass (task #1,
verifying an old PR's unconfirmed test-plan item) and **directly relevant to the crash/OOM thread**
this program exists to address: every browser-pane close leaked one CEF renderer process, invisibly
(host logs looked completely healthy). Root cause: `DestroyWindow` is owner-thread-only Win32 API;
the wrapper was created on the CEF UI thread but destroyed from a tokio IPC thread, so the destroy
silently no-op'd (`ERROR_ACCESS_DENIED`, return value never checked) on every single close since the
feature existed. **Fixed and merged (PR #2000, `docs/retro/retro-browser-pane-renderer-leak-
2026-07-07.md`)** — live-verified renderer count returning to baseline across open/close cycles,
two-panes-close-one, and mid-navigation-close scenarios. Very plausibly a material contributor to
the pagefile/commit-charge growth `SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md` investigated.

---

## 5. Recommended next step

**Pillar 1 Step 4 is the biggest remaining lift and the one with the most novel design surface** — it
deserves its own sized spec (mirroring how Step 2 and Step 3 each got one) before any code, covering
at minimum: (a) how the host/launcher distinguishes a crash-relaunch from a fresh launch (or whether
it needs to, if cold-start is made unconditionally reproject-safe instead), (b) the multi-window
recreation algorithm once `kind`/`parent_window_id` are readable (Step 3 done), (c) the in-flight
re-derivation rule with a concrete design (not just the OSDI-paper-level principle already agreed),
(d) the "Restoring session…" overlay UX already specced at the principle level in the parent design
doc §3.

**Pillar 2's WRR gap is the second-biggest lift** — smaller than Step 4 but also needs real design
(teaching the reducer about an OS-level signal it has no channel for today), not just more wiring.

Both are genuinely "write a spec first" tasks, not "proceed and implement" tasks, given this
session's repeated experience that window/lifecycle code here is deadlock-sensitive and prone to
plausible-looking-but-wrong fixes (three historical `close_browser()` attempts in April, two
iterations needed on the renderer-leak fix itself, the WRR gap being discovered only by live-testing
a supposedly-complete PR).

---

## 6. Sources

- `docs/architecture/DISCUSSION_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_06_29.md` (program index)
- `docs/specs/SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md` (corrected 2026-07-07)
- `docs/specs/SPEC_PILLAR1_STEP2_WINDOW_TOPOLOGY_PERSISTENCE_2026_07_06.md`
- `docs/specs/SPEC_PILLAR1_STEP3_WINDOW_TOPOLOGY_2026_07_07.md`
- `docs/specs/SPEC_PILLAR2_WIRE_RECONCILE_QUIT_2026_06_29.md` (corrected 2026-07-07)
- `docs/retro/retro-browser-pane-renderer-leak-2026-07-07.md`
- `docs/specs/SPEC_864_LAYOUT_SINGLE_WRITER_2026_06_30.md`
