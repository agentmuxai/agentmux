# Tab / window render architecture — coherent-frame design

**Date:** 2026-08-31
**Status:** Proposal. Not implemented. Revised after PR #2818 fixed the
tab-close flash by *optimistic removal* (§§8-9 of the 08-25 spec) — see §0.
That resolution validates this spec's thesis and **shrinks its urgency**: the
cheap structural move beat the expensive one. Read §0 before funding any phase
here.
**Owner:** unassigned
**Companion:** `docs/reports/REPORT_TAB_FLASH_SYSTEMIC_ANALYSIS_2026_08_31.md`
(the evidence and diagnosis this spec responds to — read it first)
**Scope:** `frontend/app/store/wos.ts`, `frontend/app/store/global.ts`,
`frontend/app/store/window-identity.ts`, `frontend/app/store/tab-reveal.ts`,
`frontend/app/workspace/workspace.tsx`, `frontend/app/tab/*`,
`frontend/layout/lib/{layoutModel,layoutModelHooks,layoutResize}.ts`,
`frontend/app/platform/pane-overlay.ts`,
`agentmux-srv/src/server/{wave_obj_bridge.rs,service/mod.rs}`,
`agentmux-srv/src/backend/eventbus.rs`
**Related:** `SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md` (the four-layer
fix that motivated this), `SPEC_TAB_CONTENT_REVEAL_GATE.md`,
`SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md`,
`SPEC_OBJ_UPDATE_BRIDGE_2026-05-14.md`,
`SPEC_TAB_BAR_FIRST_PRINCIPLES_2026_04_25.md`

---

## 0. What the tab-close resolution changed about this spec

PR #2818 fixed the flash with two changes: **optimistic removal** (the tab
leaves the strip when the modal opens, restored on cancel/failure — so the
strip never depends on backend ordering) and a **targeted reveal gate**
(`holdRevealGate(targetTabId)` names its destination instead of gating whoever
is active).

Three consequences for this document:

1. **The thesis holds.** The winning fix is exactly the move this spec argues
   for — make the bad state unrepresentable rather than suppress it. §8's own
   words: *"a tab that is not rendered cannot flash, whatever order the
   backend's updates arrive in."*
2. **But the cheap version won.** This spec proposed a global epoch/transaction
   system (§3.1) to make torn state impossible *everywhere*. PR #2818 got the
   same guarantee *for one surface* by decoupling it from backend ordering
   entirely — no protocol change, ~150 lines, one file. **Prefer the local
   decoupling every time it applies.** §3.1 is only justified for surfaces that
   genuinely cannot predict their own outcome.
3. **§3.4's defect was real and is now partly fixed.** The reveal gate being
   keyed on "currently active" rather than "the destination" was the confirmed
   cause of the residual pane blank. §9 fixed the targeting. The *timer-based*
   reveal (80ms settle / 800ms cap) is untouched and remains a suppressor.

**Revised recommendation:** do not fund §3.1/§3.2 on the strength of the tab
flash — that case is closed. Fund them only if a surface appears that (a) can't
predict its outcome, and (b) demonstrably tears. §3.4's causal-reveal work is
the highest-value remaining item; §3.3 is a real seam awaiting its own
evidence (it was **not** the tab-close cause — see the report's §0).

## 1. Problem statement

Roughly twenty separate flash/flicker defects have been fixed in this codebase
over ~4 months, each independently root-caused and each fixed with a *symptom
suppressor* (reveal gate, debounce, settle detector, `batch()`, emission
reordering). The most recent — the tab-close flash — absorbed **four**
consecutive correct root-cause fixes without the symptom moving.

The companion report's conclusion: the flash is an emergent property of four
architectural properties, not a bug with a root cause. This spec defines the
target architecture that removes those properties.

**Design goal, stated as an invariant:**

> **F1 (Coherent Frame).** Every painted frame reflects exactly one committed
> workspace state. No frame may show a mixture of two states, and no frame may
> show a state that was never committed.

Today F1 is *hoped for* via timers. This spec makes it *enforced by
construction*.

## 2. Current architecture (as-built)

### 2.1 The four independent pipelines

| # | Pipeline | Scheduling | Ordering guarantee |
|---|---|---|---|
| P1 | Solid reactive DOM | synchronous, batchable | within one `batch()` only |
| P2 | WOS object cache | 3 transports (bridge WS / response-broadcast WS / HTTP body) | **none across transports** |
| P3 | Layout geometry | ResizeObserver → `updateTree()` | reactive to DOM size, i.e. *after* paint |
| P4 | Native pane compositor | rAF → promise chain → async HTTP → Rust | **none w.r.t. P1** |

F1 is violable at every seam between these.

### 2.2 State fragmentation

One logical fact — "tab X closed, tab Y is now active" — lives in six places
with no shared version:

`Workspace.tabids`, `Workspace.activetabid`, the `Tab` object, its
`LayoutState`, the `Block` objects, and the **non-reactive** `layoutModelMap`
(`layoutModelHooks.ts:13`).

`updateWaveObject`'s version guard (`wos.ts:280`) is per-object, so it cannot
reject "object A from transition N+1 alongside object B from transition N."

### 2.3 The suppressor layer

`tab-reveal.ts` hides content under `visibility: hidden` for a *guessed*
duration (`SETTLE_MS = 80`, `MAX_GATE_MS = 800`) and reveals on "no long tasks
for 80ms." It is keyed on `tid === tabId()` (`workspace.tsx:65`) where
`activeTabId` derives from the same `Workspace` object whose mutation *is* the
transition — so the gate is one reactive step behind what it gates (report
§3.5).

### 2.4 Zero-size measurement

All tabs stay mounted, inactive ones `display: none` (`workspace.tsx:35-40`),
so hidden tabs measure 0×0 and every activation triggers a
measure-wrong-then-correct relayout via the container ResizeObserver
(`layoutModelHooks.ts:69` → `layoutResize.ts:216`). Existing scar tissue:
`layoutModel.ts:443`'s `Math.max(100000, …)` floor, `droppable-tab.tsx:246`'s
double-rAF re-measure.

## 3. Target architecture

Four new abstractions. They are independent — each is separately valuable and
separately shippable — but together they make F1 structural.

**Apply §0's lesson first.** Before reaching for any of these, ask the cheaper
question: *can this surface stop depending on the ordering altogether?* PR
#2818 answered yes for the tab strip and needed none of the machinery below.
The abstractions here are for surfaces where the answer is genuinely no.

**Prior committed direction, not a fresh idea.** §3.1/§3.2 continue work the
repo already scoped: `SPEC_REDUCER_SSOT_CONSOLIDATION_2026_06_22.md` generalizes
"state reconstructed or duplicated outside a single reducer authority" as a
systemic pattern, and `SPEC_864_LAYOUT_SINGLE_WRITER_2026_06_30.md` →
`SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md` already commit to strong
reducer authority over layout. Anyone picking this up should reconcile with
those rather than starting from this document. Note also
`SPEC_OBJ_UPDATE_BRIDGE_2026-05-14.md`'s open gap: `UpdateObjectMeta` for
`OTYPE_WINDOW`/`OTYPE_LAYOUT`/`OTYPE_CLIENT` **bypasses the event bus entirely**
— a fourth transport hole that §3.2's "one authoritative transport" must close.

### 3.1 `WorkspaceEpoch` — a transaction boundary over multi-object state

**Problem solved:** §2.2 fragmentation; torn state is representable today.

The reducer already produces every multi-object change as one atomic
transition. Give that transition an identity and carry it to the client.

- Backend: every reducer dispatch stamps a monotonic `epoch: u64` on **all**
  `WaveObjUpdate`s it produces. Same transition → same epoch.
- Wire: updates travel as `{ epoch, updates: [...] }`, never as bare objects.

**Completeness contract (normative — an earlier draft left this undefined):**

> **One epoch is delivered in exactly one frame, and that frame is always
> complete.** A frame carries every `WaveObjUpdate` the reducer transition
> produced. There is no such thing as a partial epoch on the wire, so the
> client never has to decide whether more is coming.

This is a *requirement on the producer*, and §3.2 is what makes it satisfiable:
the bridge must resolve every object it needs (`emit_fetched`'s async SQLite
reads) **before** emitting, then emit once. A producer that cannot assemble the
whole set must not emit a partial epoch — it must fail the transition.

Consequently the frontend apply is trivial, with **no staging buffer**:

- `wos.ts` applies each received epoch frame as one `batch()`.
- A frame whose epoch is **older** than the cell's current epoch is dropped
  (today's version guard, generalized from per-object to per-transition).

An earlier draft proposed staging partial epochs behind a bounded timeout.
That is now explicitly rejected: it cannot work without either an expected
update count or an end-of-epoch marker, and adding either buys nothing over
just requiring complete frames — while introducing a stall risk and a
timeout-tuning problem. **If a future transition genuinely cannot fit in one
frame**, this contract must be reopened deliberately and given an explicit
`{ epoch, part, final }` marker; it must not be papered over with a timeout.

**Result:** "tab deleted but workspace not yet updated" stops being a state the
UI can render. It isn't merely unlikely — it is unrepresentable.

**Cost / risk:** touches every `WaveObjUpdate` producer — that is the whole
cost, and it is the reason §0 says not to fund this on the strength of the tab
flash. With the staging buffer gone the client side is nearly trivial; the risk
sits entirely in auditing producers for the completeness contract above. Ship
behind a flag and assert the contract in dev builds (warn on an epoch whose
object set doesn't match what the reducer transition declared).

### 3.2 One authoritative transport (`P2` collapse)

**Problem solved:** §2.1's three racing transports — the direct cause of §7.

Exactly one path may drive a paint:

- **Authoritative:** the WS event stream, carrying whole epochs
  (`waveobj:batchedupdates`, already introduced by §7 — generalize it to carry
  `epoch`).
- **Demoted:** the HTTP response body's `updates` become a *cache warm* only —
  applied only if their epoch is **newer** than what the WS stream has already
  delivered (i.e. normally a no-op, as it already is in practice).
- **Removed:** the per-update fan-out in the bridge. The bridge emits **one
  epoch frame** per reducer event, never N frames. Where it genuinely needs an
  async fetch to build the frame (`emit_fetched`), it fetches *first*, then
  emits once.

This subsumes §7's parent-before-child emission ordering: with one frame per
epoch there is no intra-transition order left to get wrong. Keep the §7
ordering and its tests until the epoch frame ships — they are the correct
behaviour under the current design.

### 3.3 `PaneSurfaceSync` — a frame contract with the native compositor

**Problem solved:** §2.1's P4 seam (report §3.3) — the leading hypothesis for
the *residual* flash, and the one seam no prior fix has touched.

Today the DOM commits immediately while the native pane clip commits several
frames later via rAF → promise chain → async HTTP (`pane-overlay.ts:83-93`).

Options, cheapest first:

- **(a) Reserve-then-release.** When a modal/overlay unmounts, keep its clip
  hole registered until the host acknowledges the new clip, *then* release. Trades
  a few frames of stale-but-coherent for incoherent — strictly better under F1.
- **(b) Acknowledged clip.** `flushClip` returns a host ack; the DOM change that
  *depends* on the clip is gated on it. Correct, more invasive, adds a round
  trip to interactive paths.
- **(c) Host-side compositing of overlays.** Longest-horizon: the host composites
  the overlay itself so there is only ever one compositor. Removes the seam
  entirely; large change.

Recommend **(a)** first — it is small, reversible, and if the hypothesis is
right it should visibly move the symptom on its own.

### 3.4 `LayoutReadiness` — causal reveal, replacing timed gates

**Problem solved:** §2.3 (guessing) and §2.4 (measure-wrong-then-correct).

Replace "reveal when no long task fired for 80ms" with "reveal when the layout
model reports it has measured against real bounds."

- `LayoutModel` gains an explicit `measuredEpoch` / `isStable` signal, set when
  `updateTree()` completes against a **non-zero** container rect.
- The reveal gate consumes *that*, not a timer. `MAX_GATE_MS` survives only as
  a safety valve (with a warn when it fires — a hit means the causal signal is
  wrong and should be fixed, not tuned).
- Key the gate on the **incoming** tab id explicitly rather than on
  `tid === tabId()`, removing the one-step-behind defect (§2.3).

Optional follow-on, if measurement shows it matters: give hidden tabs
`content-visibility: hidden` or keep them sized-but-clipped so their containers
retain real dimensions, eliminating the 0×0 measurement class outright. This
would let `layoutModel.ts:443`'s `Math.max(100000, …)` floor and
`droppable-tab.tsx:246`'s double-rAF be deleted — a good proxy for whether the
class is genuinely gone.

## 4. What gets deleted

A design is only structural if it *removes* the suppressors. On completion:

- `tab-reveal.ts`'s hand-rolled long-task detector → replaced by §3.4's causal signal
- `MAX_GATE_MS` / `SETTLE_MS` as behaviour → demoted to instrumented safety valve
- `layoutModel.ts:443`'s zero-rect floor → unnecessary (§3.4 follow-on)
- `droppable-tab.tsx:246`'s double-rAF re-measure → unnecessary (§3.4 follow-on)
- §7's parent-before-child bridge ordering → subsumed by one-frame-per-epoch (§3.2)

If a phase lands and deletes nothing, that phase did not do its job.

## 5. Phasing — measurement first

> **Revised per §0.** The tab-close case is closed; do not run this phasing for
> it. What follows applies to the *next* member of this class. Phase 0 is the
> part that generalizes — and PR #2818's own writeup shows why: none of §§5-7
> was ever tested against a build containing all of them, so several "still
> broken" reports were of incomplete builds. Reasoning ran four fixes deep
> without one clean observation.

**Phase 0 — Instrument, and verify the build under test. Blocking.**

Before any further change:

- **Confirm the build actually contains the fix** (label/commit check). This
  alone would have saved two of the five tab-close rounds.

- A dev-only frame log that timestamps: each WOS epoch/update application, each
  `activeTabId` flip, each `tabSwitching` transition, each `updateTree()`, each
  `flushClip()` send **and host ack**.
- Capture the affected gesture end to end, on a build confirmed to contain
  whatever fixes are already believed to be in it.
- **Deliverable:** the ordered list of what actually painted, from the gesture
  to the settled frame. **Rank hypotheses on that list, not on narrative fit** —
  the report's §0 records a case where an inferred mechanism was ranked above
  an already-confirmed defect, and was wrong.

Phase 0 is cheap and is the only step that can *falsify* a hypothesis before
it is built on.

**Phases 1-3 are deliberately unordered.** The tab-close case is closed, so
there is no live symptom to sequence them against; ordering should come from
Phase 0's trace of whatever the next symptom turns out to be.

- **`PaneSurfaceSync`** (§3.3) — a real unsynchronized seam with precedent
  elsewhere, but **not** the tab-close cause. Do it when a trace implicates it.
- **`WorkspaceEpoch`** (§3.1) + **transport collapse** (§3.2) — the structural
  core, and the most expensive. Per §0, justified only for a surface that
  cannot predict its own outcome *and* demonstrably tears. Ship behind a flag;
  assert the §3.1 completeness contract in dev builds.
- **`LayoutReadiness`** (§3.4) and deletion of the suppressor layer (§4) — the
  highest-value remaining item per §0, since the timer-based reveal (80ms /
  800ms) survives untouched and still guesses at readiness.

## 6. Non-goals

- Not a rewrite of the reducer, the WOS cache, or TileLayout. Every proposal
  here is additive-then-subtractive on existing structures.
- Not a change to the all-tabs-stay-mounted policy (xterm.js scrollback depends
  on it) — §3.4's follow-on changes how hidden tabs are *sized*, not whether
  they are mounted.
- Not a performance project. F1 is about *coherence*; if a phase trades a few
  frames of latency for a coherent frame, that is the intended trade.

## 7. Open questions

1. ~~**Is §3.3 actually the residual cause?**~~ **Answered — no.** PR #2818
   fixed the tab-close flash without touching the native compositor (§0, and
   the report's §0 scorecard). Do **not** re-run this investigation for that
   symptom. The open form of the question is now: *when a future flicker is
   independently observed on an overlay-heavy path, is the `sendClip` → rAF →
   async-HTTP seam its cause?* — a question for that symptom's own Phase 0,
   with §3.3's documented precedent (`REPORT_NEW_WINDOW_STARTUP_COLOR_FLASH_2026_07_14.md`,
   `SPEC_BROWSER_PANE_LOADING_INDICATOR_FLICKER_2026_08_17.md` Cause 2) as
   prior art rather than as evidence.
2. ~~**Can a partial epoch occur?**~~ **Resolved by fiat in §3.1** — the
   completeness contract forbids it on the wire, and the staging buffer is
   dropped. The residual question is an audit, not a design one: *does every
   `WaveObjUpdate` producer already assemble its full set before emitting?*
   The bridge's `emit_fetched` path is the one known to need restructuring.
3. **Do LAN/multi-window renderers need epoch coordination**, or is per-renderer
   monotonicity enough? Suspect the latter; unverified.
4. **Does the confirm modal need to exist on this path at all?** The gesture is
   reversible (tabs are restorable). Removing the modal would sidestep the P4
   seam for *this* gesture — though not for menus, dropdowns or any other
   overlay, so it is a mitigation, not a fix.
