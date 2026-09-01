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

- Backend: every reducer **dispatch** stamps a monotonic `epoch: u64` on **all**
  `WaveObjUpdate`s it produces. Same dispatch → same epoch.
- Wire: updates travel as `{ epoch, updates: [...] }`, never as bare objects.

**The unit is the DISPATCH, not the event.** This is the subtlety an earlier
draft got wrong. `dispatch_to_reducer` returns a `Vec<Event>`; `publish_events`
sends each element independently; the bridge consumes one event at a time. One
*command* routinely produces several *events* — `DeleteWorkspace` emits
`WorkspaceDeleted` plus one `SrvWindowClosed` per affected window. Stamping and
emitting per-event therefore splits a single logical transition across several
frames and reopens exactly the intermediate paint the epoch exists to prevent.

So the producer change is not "stamp each event" but **publish a dispatch
envelope**: `publish_events` must carry the whole `Vec<Event>` from one dispatch
as a unit, and the bridge must project that envelope into one frame. Any design
that keeps the bridge subscribed to individual events cannot satisfy the
contract below, however the stamping is done.

**Completeness contract (normative):**

> **One epoch is delivered in exactly one frame, and that frame is always
> complete.** A frame carries every `WaveObjUpdate` produced by one reducer
> dispatch. There is no partial epoch on the wire, so the client never has to
> decide whether more is coming.

**Gaps, not just order — the client needs a watermark.** Dropping a frame whose
epoch is *older* than current state is safe; a **missing** frame is not. These
frames are deltas, not snapshots: `EventBus::try_send_lane` drops priority
events when a lane is full, and `run_wave_obj_bridge` continues past a
`Lagged` without resyncing. If epoch N changed object A and the client next
receives N+1 changing B, applying N+1 paints old A beside new B — a state that
was never committed, which is an F1 violation by the same definition the tab
flash was.

The apply contract is therefore:

- `wos.ts` tracks a **contiguous epoch watermark** — the highest epoch `E` such
  that every epoch up to `E` has been applied.
- A frame at `watermark + 1` applies as one `batch()`; the watermark advances.
- A frame **older** than the watermark is dropped (today's version guard,
  generalized from per-object to per-transition).
- A frame **newer** than `watermark + 1` means at least one epoch was lost. The
  client must **not** apply it. It requests an authoritative resync (a snapshot
  of the affected objects, or a full workspace re-read), adopts the snapshot's
  epoch as its new watermark, and resumes. Loud telemetry on every resync — a
  frequent resync means the transport is lossy and that is the bug to fix, not
  the resync path.

Without the resync arm, this design is strictly *worse* than today's
per-object version guard under packet loss, because it would confidently apply
a delta onto a base it never received.

**Failure to assemble a frame is a resync, not a rollback.** An earlier draft
said a producer that cannot assemble the full set "must fail the transition."
That is unimplementable where it was written: service handlers apply reducer
events to SQLite and publish them *before* the asynchronous bridge performs its
reads, so by the time an `emit_fetched` read fails the mutation is already
committed and acknowledged. Fetching-before-emitting prevents a *partial* frame
but converts that failure into a *missing* epoch sitting behind a committed
mutation. Two admissible resolutions, and a design must pick one explicitly:

1. **Assemble before commit** — the frame is built and validated as part of the
   dispatch, so a failure can genuinely abort the transition. Strongest, and the
   most invasive: it puts object reads on the commit path.
2. **Assemble after commit, heal by watermark** — accept that a producer failure
   yields a gap, and let the client's watermark detect it and resync (above).
   Cheaper, and it reuses machinery the lossy-transport case already requires.

(2) is recommended: it needs no change to the commit path, and the resync arm
is non-optional anyway because `try_send_lane` can drop frames regardless of
producer behaviour.

**Staging buffers remain rejected.** An even earlier draft proposed staging
*partial* epochs behind a bounded timeout. That cannot work without an expected
count or an end-of-epoch marker, and buys nothing over requiring complete
frames — while adding a stall risk and a timeout to tune. Note this is a
different mechanism from the watermark above: the watermark tracks *whole
epochs*, never fragments of one. If a future transition genuinely cannot fit in
one frame, reopen the contract deliberately with an explicit
`{ epoch, part, final }` marker rather than a timeout.

**Result:** "tab deleted but workspace not yet updated" stops being a state the
UI can render — provided the dispatch envelope, the watermark, and the resync
arm all exist. Any one of the three missing and the guarantee is only
probabilistic, which is what the four failed tab-flash fixes already were.

**Cost / risk:** three distinct pieces of work, not one — a dispatch envelope
through `publish_events` and the bridge, an epoch watermark in `wos.ts`, and an
authoritative resync endpoint. That is substantially more than the earlier draft
implied, and it reinforces §0: do not fund this on the strength of the tab
flash, which was closed for ~150 lines by decoupling one surface instead.
Ship behind a flag; assert in dev builds that a dispatch's emitted object set
matches what the reducer declared, and alarm on resync frequency.

### 3.2 One authoritative transport (`P2` collapse)

**Problem solved:** §2.1's three racing transports — the direct cause of §7.

Exactly one path may drive a paint:

- **Authoritative:** the WS event stream, carrying whole epochs
  (`waveobj:batchedupdates`, already introduced by §7 — generalize it to carry
  `epoch`).
- **Demoted:** the HTTP response body's `updates` become a *cache warm* only —
  applied only if their epoch is **newer** than what the WS stream has already
  delivered (i.e. normally a no-op, as it already is in practice). Note it can
  never *fill a gap*: a body arriving at `watermark + 2` is as unapplicable as
  the WS frame was, and must fall through to the same resync.
- **Removed:** the per-update fan-out in the bridge. The bridge emits **one
  epoch frame per reducer DISPATCH** — not per event (§3.1: one command can
  emit several events). This is the change that requires `publish_events` to
  carry a dispatch envelope and the bridge to subscribe to envelopes rather
  than to individual events; without it, the rest of §3.1 cannot hold. Where
  building the frame needs async fetches (`emit_fetched`), it resolves them all
  *first*, then emits once.

This subsumes §7's parent-before-child emission ordering: with one frame per
dispatch there is no intra-transition order left to get wrong. Keep the §7
ordering and its tests until the epoch frame ships — they are the correct
behaviour under the current design.

**Scope note.** "Exactly one path may drive a paint" is about the *update*
transports. It does not cover the resync path §3.1 requires, which is a fourth
path by construction — a client-initiated authoritative read. That is
acceptable because a resync delivers a *snapshot*, not a delta: it cannot tear
against the stream, it can only be stale, and its adopted epoch re-anchors the
watermark. Any resync design that returned deltas would reintroduce the problem.

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
   dropped. The residual questions are now scoped and concrete:
   a. *Can `publish_events` carry a dispatch envelope without disturbing its
      other subscribers* (the persist subscriber, the disk writer)? They
      consume individual events today and would need to either flatten the
      envelope or move to it.
   b. *What is the authoritative resync?* A per-object snapshot read, a
      workspace-scoped one, or a full reload. Cheapest correct option wins;
      it only has to be a **snapshot**, never a delta (§3.2 scope note).
3. **Do LAN/multi-window renderers need epoch coordination**, or is per-renderer
   monotonicity enough? Suspect the latter — each renderer's watermark is its
   own, and a resync is client-initiated — but unverified, and the answer
   changes if two renderers ever have to agree on a *rendered* frame rather
   than just on state.
4. **Is the watermark's resync arm worth the complexity at all**, or is the
   honest conclusion that per-object versioning (today's behaviour) is the
   right trade for a lossy local transport? A gap-tolerant delta protocol is a
   real distributed-systems problem; §0 already argues the tab flash did not
   justify paying for one. This question should be answered *before* §3.1 is
   funded, not during.
5. **Does the confirm modal need to exist on this path at all?** The gesture is
   reversible (tabs are restorable). Removing the modal would sidestep the P4
   seam for *this* gesture — though not for menus, dropdowns or any other
   overlay, so it is a mitigation, not a fix.
