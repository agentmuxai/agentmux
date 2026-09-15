# REPORT — Splash screen "looks stuck": why the gaps exist, and a DRY/ordered redesign

**Date:** 2026-09-14
**Author:** Agenty (investigation), synthesizing a fresh code audit against two
prior docs
**Status:** Diagnosis + proposed redesign. Nothing implemented in this pass —
report only, per the ask.
**Scope:** The native startup splash (`agentmux-launcher/src/splash.rs`
[Windows], `splash_mac.rs` [macOS], `splash_linux/{mod,x11,wayland}.rs`
[Linux]) and everything that feeds it (`startup_events.rs`,
`agentmux-cef/src/lib.rs`, `agentmux-cef/src/client/navigation.rs`,
`agentmux-launcher/src/{srv_spawner,supervisor/windows,supervisor/unix}.rs`).
**Not in scope:** the shutdown countdown modal
(`SPEC_SHUTDOWN_COUNTDOWN_MODAL_2026_09_04.md`) — a different, unrelated
feature that happens to share the word "splash" in some doc titles.

---

## 0. The ask, restated precisely

> "there are a lot of gaps (where no timer is progressing) it looks stuck.
> ideally we want each to be continuous with the final tally at the end
> adding up exactly... it sounds like the architecture may need a rethink,
> perhaps made DRY and ordered better"

Two prior investigations already exist and are not re-derived here, only
verified against current code and extended:

- `docs/analysis/ANALYSIS_SPLASH_SCREEN_TIMING_2026_07_20.md` (2026-07-20) —
  diagnosed macOS's count-up race and the total-vs-items gap, macOS only.
- `docs/retro/retro-splash-screen-rows-dont-count-up-2026-07-22.md`
  (2026-07-22) — confirmed PR #2244 fixed the gap-accounting complaint but
  not the count-up complaint, on macOS.
- `docs/reports/REPORT_SPLASH_TO_FIRST_PAINT_BLANK_WINDOW_GAP_2026_09_03.md`
  (2026-09-03/04) — fixed the blank-window-after-splash defect and added a
  `paint` stage, on Windows + Linux, explicitly not macOS.

None of the three treated the splash as one system across all three
platforms. That turns out to be exactly the gap: **the "stuck" feeling you're
seeing isn't one bug, it's the emergent result of three independently-grown
implementations that have each fixed a different subset of the same
underlying defects.** The rest of this report establishes that with
file-and-line evidence, then proposes a redesign.

---

## 1. Root cause: three reimplementations, not three platforms

`startup_events.rs` (115 lines) defines exactly four event variants
(`StageBegin`/`StageEnd`/`SubBegin`/`SubEnd`) and a channel sink. That is
**the entire shared surface area.** Everything downstream of it — the row
model, the tick loop, the "is this row still running" logic, duration
formatting, truncation, and the total/reconciliation math — is written three
separate times:

| | Windows `splash.rs` | macOS `splash_mac.rs` | Linux `splash_linux/{mod,x11,wayland}.rs` |
|---|---|---|---|
| Size | 668 lines | 1,499 lines | 434 + 265 + 267 = 966 lines |
| Redraw cadence | 16ms full drain+composite loop | 8ms tick, `apply_tick` | 16ms (`FRAME_MS`) drain loop |
| Same-tick Begin+End deferred (count-up fix)? | **No** | **Yes** (`apply_tick`, 5 unit tests) | **No** |
| Total row | Bare `"total: {}"`, no reconciliation | `other_ms = total - accounted`, reconciled | **No total row at all** |
| Sub-item handling | shown | shown | **only the last sub-item per stage is shown** (`s.subs.last()`) — earlier ones silently dropped |
| "Hold" phase semantics | freeze → pause `hold_ms` → fade, no redraw during pause | freeze → pause `hold_ms` → fade, no redraw during pause | **no freeze/pause at all** — single combined gate, animates through the fade |
| Frozen-row-if-End-never-arrives bug | **Present** | **Present** | **Absent** (never stops redrawing, so never freezes — different tradeoff, not a fix) |

Every one of these is a place where a fix landed on one file and simply
never got ported to the siblings, because there is no shared implementation
for a port to land *in*. This is why the July analysis's fix (count-up) and
the September report's fix (paint stage, blank-window gate) each closed a
real bug on 1–2 platforms while leaving the identical bug live on the
others — not because anyone missed a spot, but because the architecture
makes "fix it in three places" the only available shape a fix can take, and
that has never actually happened for any of these issues at the same time.

**This is the "make it DRY" half of your ask, and it is not cosmetic — it is
the mechanism producing the specific symptom you're seeing.** A user running
Windows or Linux today gets rows that snap instead of counting up (the
"already fixed" bug, still live on 2/3 platforms) *and* a row that can freeze
mid-animation and never recover if its End event is delayed past `ready_at`.
Both look exactly like "a timer that stopped progressing" — because that is
literally what they are.

---

## 2. Root cause: real, un-instrumented time — not just an animation bug

Even a perfectly DRY, perfectly animated implementation would still show
gaps, because real elapsed time exists today that no stage's Begin/End pair
covers on *any* platform. Walking the actual boot sequence:

```
launcher start → prep → backend (srv spawn + migrations, in-process) →
host spawn → [dlopen, macOS only] → cef_init → window creation →
on_load_end → paint → frontend bootstrap → tab reveal
```

| Transition | Covered by a stage? | Notes |
|---|---|---|
| Launcher start → `prep` begin | **Windows: no stage exists at all.** macOS/Linux: yes. | `windows.rs` has no `"prep"` string anywhere — confirmed by grep. Structural: the splash doesn't exist yet at this point on Windows. |
| Inside `backend`: real DB migrations | **No.** | See §2.1 — this is the largest single gap and it's real in production. |
| `host` end → `on_load_end` | **No**, any platform. | `host` ends at "process spawned," not at the host's own internal dlopen→cef_init→window→load sequence. On Windows this span can include a real, documented up-to-30s Windows Defender first-run scan (`windows.rs:506-544`). |
| `dlopen` → `cef_init` | **Windows/Linux: no stage at all** (`dlopen` is macOS-only, `#[cfg(target_os = "macos")]`). | Whatever framework-load cost exists on those platforms is invisible. |
| `cef_init` end → window creation → `on_load_end` | **No**, any platform. | Real window/browser construction + initial navigation start. |
| `on_load_end` → real first paint | **Windows + Linux: yes** (`paint` stage, Sept report). **macOS: no.** | Confirmed directly: macOS's `reveal_top_level_window` has no gated path; goes straight to unconditional `window.show()`. The Sept report measured this gap at ~1–2s on Windows/Linux; nothing suggests macOS is different, it's just unmeasured. |
| `paint` end → frontend bootstrap → store hydration → tab/pane mount | **No**, any platform. | Zero references to `sendLauncherMsg`/`startup_stage_begin`/`startup_stage_end` anywhere in `frontend/`. Spec'd in the original June design (step 7) and never built — confirmed independently by two prior docs and this pass. |
| Tab-reveal settle window (up to 800ms) | **No, and structurally can't be** — it now happens *after* the native splash has already dismissed. | The ready-signal that ends `paint` and dismisses the splash fires together (`navigation.rs`'s `signal_gated_reveal_complete`), so this window is outside the splash's lifetime by design, not by oversight. |

### 2.1 The single biggest gap: migrations are invisible, and the code path that would show them is dead

`srv_spawner.rs`'s `run_migrate` function — the one with a `migrations`
stage and per-migration sub-item rows — has its own doc comment stating
*"Not called during normal startup... has no active callers"* and is marked
`#[allow(dead_code)]`. Grep confirms zero call sites outside its own
definition and tests. **The entire "S2 — Migrations" expandable sub-list
design only ever fires in the `--splash-selftest` fixture.**

Real migrations run in-process inside srv (`bootstrap.rs`'s
`run_pending_migrations`), entirely inside the already-open `backend` span.
The only cross-process signal is `AGENTMUXSRV-MIGRATING migrations:<n>` on
stderr, and its only effect is extending a timeout — it never becomes a
splash row. So today, a user who hits a genuinely slow migration (the code's
own comment allows for up to 30 minutes) sees a plain `Backend startup ▶
N.Ns...` counting up with **zero indication anything unusual is happening**.
This is the single largest contributor to "looks stuck" in the whole system,
present on every platform, in real (non-fixture) builds — not a cosmetic
animation gap but a genuine information gap.

### 2.2 A concrete, already-shipping "frozen forever" row

Not a hypothetical race: `windows.rs`'s `first-run-wait` sub-row (a 5s-delay
diagnostic for the Defender-scan gap) has **no matching `sub_end` call
anywhere in the codebase.** Whenever it fires, it is guaranteed to sit frozen
in "running" state for the rest of the boot, attached to a parent `host` row
that has *already* shown as done. This is the cleanest live reproduction of
the "frozen row" defect described in §1 — it doesn't need bad timing to
happen, it happens by construction every time the sub-row is created at all.

---

## 3. Why "the final tally adds up exactly" is not true today, structurally

The "total" is not a sum of the visible rows on any platform — it's an
independent wall-clock stopwatch (`Instant::now() - start`, measured from
splash-window creation to the ready signal) with, by the July analysis's own
words, "zero arithmetic relationship" to any stage's `duration_ms`. Given
§2's real gaps, that's necessarily true — the total includes real time no
stage covers, so summing the stages can never equal it without either (a)
an explicit catch-all bucket or (b) actually closing every gap.

- **macOS** does (a): a post-hoc `other_ms = total - accounted` row, computed
  once, after the fact, frozen at that value. It reconciles arithmetically
  but doesn't explain *what* the uncovered time was — "other" is a label for
  ignorance, not a stage.
- **Windows** does neither — a bare `total:` annotation with no reconciliation
  check at all; the gap is real but invisible even as a number.
- **Linux** has no total row of any kind.

So today, "the tally adding up exactly" is true on exactly one of three
platforms, and even there only because of a fudge-factor bucket rather than
real accounting.

---

## 4. Proposed redesign

Ordered by dependency — each phase is buildable and shippable on its own,
and later phases assume earlier ones landed.

### Phase 1 — Extract one shared `splash_core` (the DRY fix)

Pull the row model, tick/apply logic (with the same-tick-deferral fix
generalized from macOS's `apply_tick`), duration formatting, truncation, and
total/other reconciliation into a single crate-internal module used by all
three platform backends. Each platform backend keeps *only* what is
genuinely platform-specific: the window/surface creation, the actual pixel
compositing call, and the event pump (`pump_app_events` on macOS, the X11/
Wayland event loop, Win32's message loop). Everything about "what does this
row currently say and what color is it" becomes one implementation, one set
of tests, and one place future fixes land — instead of the three-times
divergence in §1's table.

This alone is what makes every fix below a **one-time** fix instead of a
per-platform chore, and it directly retires PLAN item #5 (§B.4.3), open
since July and, per this audit, *more* divergent now than when it was first
logged.

**Also fold into `splash_core`:** a single, explicit hold-phase state
machine with one clearly-named semantic — see §4.4 — replacing the two
different meanings currently sharing the `AGENTMUX_SPLASH_HOLD_MS`/
`min_hold()` name across platforms.

### Phase 2 — Make every row finalize, never freeze mid-state

Decouple redraw from "a new event arrived." Concretely: once `splash_core`
exists, drive its redraw on a **fixed timer for the splash's entire
lifetime** (not "timer-driven pre-ready, event-driven post-ready" as today),
*and* at the moment `ready_at` is set, walk every row still in `done ==
None` and explicitly finalize it (e.g. "interrupted"/"—" status, distinct
color) rather than leaving it live-styled with a value that will never
change again. This closes both the general race (§1's table) and the
concrete shipping instance (§2.2's `first-run-wait` row) in one mechanism,
without needing every emitter to remember to always send a matching End —
which `first-run-wait` proves is not a safe assumption to build on anyway.

### Phase 3 — Close the real gaps, in order of size

1. **Wire real migrations into the splash.** Not the dead `run_migrate`
   subprocess path — bridge the existing in-process
   `AGENTMUXSRV-MIGRATING migrations:<n>` stderr signal (already parsed,
   already extends the timeout) into `sub_begin`/`sub_end` calls against the
   `backend` stage, so a slow migration shows real sub-rows instead of a
   silent multi-minute clock. This is §2.1's gap — the single largest
   "looks stuck" contributor — and needs no new cross-process protocol,
   only wiring the signal that already exists.
2. **Extend `paint` to macOS.** The gate and stage already exist and are
   proven on two platforms; porting is mechanical relative to inventing it
   was. Closes the largest remaining single-platform asymmetry.
3. **Add a stage for `host`-spawned → `on_load_end`** (currently silent on
   all platforms), or at minimum fold the documented Windows Defender
   first-run gap into a real, terminating sub-row rather than the orphaned
   one in §2.2.
4. **Add a Windows `prep` stage.** Structural blocker noted in July remains
   real (no splash exists yet at that point) — either move splash creation
   earlier on Windows, or accept and document this as a permanent asymmetry
   rather than a silently-still-open item.
5. Frontend bootstrap → tab reveal (§2's last two rows) is a **judgment
   call, not a mechanical gap-close** — see §4.5.

### Phase 4 — Make the total a derived value, not a second stopwatch

Once Phase 3 has closed the large gaps, define `total` as `sum(stage
durations) + other`, computed and displayed identically on all three
platforms via `splash_core`, with `other` always shown (even at 0/near-0)
rather than only appearing on macOS. This is what makes "the final tally
adds up exactly" a property guaranteed by construction — the arithmetic
can't drift because there's only one code path computing it — rather than
something that happens to hold on one platform today because of a
post-hoc subtraction.

### 4.4 Decide, once, what "hold" means

Today `AGENTMUX_SPLASH_HOLD_MS`/`min_hold()` answers two different
questions depending on platform: "minimum total time the window stays
visible" (Linux) vs. "pause duration after the summary is ready, before
dismissing" (Windows/macOS — which also independently converged on `2000`ms
as their actual default, not the original spec's documented `3000`, a
drift worth deciding to keep or correct explicitly rather than carrying
forward as an accident). Pick one semantic as the actual product intent and
implement it once in `splash_core`; this removes the last source of
platform-specific "why does this one never pause to let me read it"
divergence (Linux today never freezes a completed summary for the user to
read at all).

### 4.5 Frontend bootstrap / tab-reveal window — recommend against folding into the native splash

This is the one gap in §2's table this report recommends *not* closing by
extending the native splash's lifetime. The in-page "brain" overlay already
exists specifically to cover this window, and holding an OS-native splash
window open across the handoff to a same-purpose in-page indicator is a
real architectural change (cross-process lifetime coordination, a second
render surface staying alive concurrently) for a phase that already has a
purpose-built indicator, just not a duration-labeled one. If this window's
opacity (no elapsed-time readout) turns out to matter in practice, the
lower-risk fix is giving the existing brain overlay its own lightweight
timer label, not extending native-splash lifetime to reach it.

---

## 5. What this buys, concretely

- Every row on every platform gets at least one "running" frame before it
  can complete — no more snap-to-final on Windows/Linux (Phase 1).
- No row can be left permanently frozen mid-animation, including the
  `first-run-wait` case that ships today (Phase 2).
- The single largest actual "stuck" experience — a real migration running
  behind a static "Backend startup" clock — gets real sub-progress (Phase 3.1).
- macOS stops being the one platform where a 1–2s blank-feeling wait after
  the splash's own timeline says "done" is entirely unaccounted for
  (Phase 3.2, closing the one asymmetry the Sept report deliberately left
  open).
- "Sum of rows equals total" becomes true by construction, on all three
  platforms identically, instead of true on one platform via a fudge factor
  and untrue (or unmeasured) on the other two (Phase 4).
- Every future fix (a new stage, a new formatting tweak, a new hold
  semantic) lands once instead of needing to be independently ported three
  times and, per this audit's own findings, reliably not being.

## 6. What this report does not resolve

- Exact values for the unified hold semantic (§4.4) and whether `2000ms`
  (what two platforms actually ship) or `3000ms` (what the original spec
  says) is the intended product behavior — needs a product decision, not an
  engineering one.
- Whether frontend-bootstrap telemetry (§4.5) is worth building into the
  brain overlay — flagged as a recommendation, not decided here.
- PLAN items #6 (IPC-signal dismiss instead of ready-file polling) and #8
  (srv/host spawn parallelization) — both still open, both orthogonal to the
  "looks stuck" complaint this report addresses, not re-scoped here.

---

## 7. References

- `docs/analysis/ANALYSIS_SPLASH_SCREEN_TIMING_2026_07_20.md`
- `docs/retro/retro-splash-screen-rows-dont-count-up-2026-07-22.md`
- `docs/reports/REPORT_SPLASH_TO_FIRST_PAINT_BLANK_WINDOW_GAP_2026_09_03.md`
- `docs/specs/PLAN_SPLASH_TELEMETRY_OPEN_ITEMS_2026_07_22.md`
- `docs/specs/SPEC_SPLASH_STARTUP_TELEMETRY_2026_06_25.md` (original design;
  "Running Clock" and "Summary Hold" sections)
- `docs/specs/SPEC_MIGRATION_FRAMEWORK_2026_06_24.md` (confirms
  `AGENTMUXSRV-MIGRATING` is a deadline-extension-only signal today)
- Code: `agentmux-launcher/src/{startup_events,splash,splash_mac,main}.rs`,
  `agentmux-launcher/src/splash_linux/{mod,x11,wayland}.rs`,
  `agentmux-launcher/src/{srv_spawner,supervisor/windows,supervisor/unix}.rs`,
  `agentmux-cef/src/lib.rs`, `agentmux-cef/src/client/navigation.rs`,
  `agentmux-cef/src/launcher_ipc/reporters.rs`,
  `agentmux-common/src/ipc.rs`
