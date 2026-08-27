# Architecture Analysis: Why the Agent Pane Keeps Regressing (Activity Dock Flicker as a Case Study)

**Date:** 2026-08-25
**Trigger:** repo owner: *"we are regressing on different elements, especially
regarding the agent pane... it sounds like we may need an architecture rethink
or modularization. for example, we still see flickers of old long-running task
docks when the agent pane loads. why is that, after all the work we did? it
sounds like perhaps we have spaghetti that needs to be cleaned up."*
**Method:** three independent deep-dive investigations (git archaeology across
every retro/report/spec touching this symptom family; live read of the current
Activity Dock data-flow architecture, frontend and backend; a structural survey
of the whole `frontend/app/view/agent/` module and its prior architecture
assessments), plus direct verification of the current release/version state.
**Scope:** analysis only, per the request — no code changes. Recommendations at
the end are unimplemented, for discussion.

---

## TL;DR

Two things are simultaneously true, and they should not be conflated:

1. **The specific flicker you're describing very likely has an already-merged
   fix sitting unreleased.** `VERSION_HISTORY.md`'s top entry on `main` is
   already `0.55.24` and includes *"fix(agent): gate the BrainSpinner on
   subagent backfill so the Activity Dock never flickers"* (PR #2781,
   2026-08-24) — but **no tag newer than `v0.55.21` has ever been pushed.**
   The only real, downloadable, published build is 33+ commits and three
   patch versions behind `main`. If you're running the actual released
   AppImage/portable, you are, by construction, running a build from before
   this exact fix (and before the debounce fix, and before the shell-flash
   fix). This alone would fully explain "we did all this work and I still see
   it" without any deeper architecture problem at all. **§1 covers this and
   the recommended immediate action.**

2. **Independently of the release gap, there is a real, recurring architectural
   pattern behind this entire bug family — not just an unlucky string of
   isolated bugs.** At least 10 distinct incidents in this exact symptom
   family have shipped fixes over the last 5 weeks, concentrated in the same
   ~2,100-line `activity/` subdirectory and its counterpart backend
   subsystems. The root cause, confirmed independently by two different
   investigation angles, is narrow and specific: **the backend never tells
   the frontend whether an event is a historical replay or a genuinely live
   update**, and this one missing piece of information has already produced
   at least two structurally identical bugs (a subagent-storm flicker and a
   shell-status flicker), each "fixed" with its own bespoke, independently-
   invented workaround instead of once, generically, at the source. On top of
   that, the specific frontend modules behind the dock (`subagent-source.ts`,
   `dispatch-source.ts`) were built in June 2026 — a month *after* this
   codebase's own architecture team audited the agent pane as "100%
   reducer-routed" and blessed a specific, disciplined state-management
   pattern — and were built entirely *outside* that discipline, missing
   exactly the settlement/audit guarantees that pattern exists to provide.
   **§2-4 cover this in full, with the complete incident timeline.**

Neither finding excuses the other. Cut a release now to get the already-fixed
bugs into users' hands (§1), *and* treat the architectural gap as real,
scoped, tractable work (§5's recommendations) — not a full "rethink," but not
nothing either.

---

## 1. The mundane explanation: check this first

Confirmed directly against the repo:

```
$ git tag --sort=-creatordate | head -3
v0.55.21
v0.55.18
v0.55.14

$ git rev-list v0.55.21..main --count
33

$ head -3 VERSION_HISTORY.md
# AgentMux Version History
## 0.55.24 — 2026-08-24
```

`VERSION_HISTORY.md`'s current top section (version 0.55.24, dated 2026-08-24)
already includes, verbatim:

- `fix(agent): gate the BrainSpinner on subagent backfill so the Activity Dock
  never flickers` (PR #2781) — the most recent, most targeted fix in the
  entire incident chain below.
- The 2026-08-23 debounce-coalescing fix (PR #2773) shipped even earlier, in
  the still-unreleased `0.55.22`.

**The actual published GitHub Release is `v0.55.21`.** `v0.55.22`'s own
release commit exists on `main` (`01cb05d3e`) but was never tagged; a later
`chore: release v0.55.24` commit superseded it, also untagged. Nothing past
`v0.55.21` has ever been built and distributed as an actual downloadable
artifact.

**Recommended immediate action, separate from anything below:** cut a real
release now (`task release` already computed through `0.55.24`; tag and
publish it) so the fixes that already exist reach an actual running build.
This is a five-minute action that may resolve the reported symptom on its own,
and doing it first will make it much easier to tell whether any *further*
flicker you observe afterward is a genuinely new (fifth) incident in this
family, or just the release lag catching up.

---

## 2. Full incident timeline

Ten distinct incidents in this exact symptom family (docks/subagent rows/shell
rows appearing then vanishing or misreporting on load), spanning 2026-07-17 to
2026-08-24 — about 5 weeks:

| Date | PR / Commit | Symptom | Root cause | Fix scope | Deeper fix left undone? |
|---|---|---|---|---|---|
| 07-17 | Fix A (subagent_watcher) + companion launcher Fix B | Whole-app OOM crash ×3 on cold restart | Unbounded historical replay: every subagent JSONL ever written for a session was replayed on every reopen/restart | **Narrow.** Capped replay to `BACKFILL_MAX_FILES=200` | Yes — corpus never pruned; Unix/macOS srv has no OOM-retry parity at all |
| 07-23 | PR #2286 | A subagent's activity leaked into 5 unrelated, already-closed panes | fs watcher not scoped to the calling agent's own files; leaked watchers never torn down on ungraceful close | **Structural, narrowly scoped** to this one leak | A *different* watcher-scoping bug (wrong config dir) recurred 4 weeks later (08-22 row below) |
| 07-24 | PR #2293 | Error rows in the dock accumulated forever | `RETENTION_MS.error` was `Infinity` by original design | **Narrow.** 15s retention + flash animation | Yes — 4 independent near-identical "flash" implementations exist codebase-wide with no shared primitive |
| 08-06 | (root cause hypothesized, fix status unclear) | Phantom "Agent" placeholder rows / ghosts in Swarm tree | Hypothesized: tree-builder renders any `parent_block_id` referenced by a subagent record even with no real registered block | **Unclear — retro explicitly says "NOT yet live-confirmed" at time of writing** | Yes, by definition — worth confirming whether this was ever actually closed |
| 08-10 | PR #2519 + #2520 | 17 dock rows stuck "running" for hours after backgrounded shells finished | `run_in_background:true` trusted as proof of a detached process; harness sometimes resolves synchronously instead | **Narrow, well-scoped** | Yes, twice — `bg` visibility silently expires after 1h with no refresh; no automated pre-push staleness check |
| **08-22** | **PR #2770** | **Shell rows flash "running" on pane load, then vanish** | `shell_node_create` replays verbatim on every mount with no status field; the correction (an "exit" event) arrives via a slower, independent round trip | **Narrow** — added a fast synchronous status-correction RPC. Took 3 review rounds, each catching a new race the fix itself introduced | Yes, explicitly — flash window narrowed, not eliminated by a hard guarantee; a "pending" status variant or WPS-replay rework judged "disproportionate" |
| 08-22 | (spec, implemented) | Subagent watcher watched the wrong config dir for identity-bound agents | Config dir resolved once at launch, diverges from the real per-turn dir | **Structural for this case** | Yes — no backfill/refresh if identity rebinds mid-session |
| 08-22/23 | PR #2761/#2768, then same-day fix | Pane flickers through spinner→picker→content on tab actions; then (fix regression) spinner got stuck **permanently visible** | No reveal gate for pane-local mounts, only whole-tab switches; the fix's own cross-fade rewrite had a construction-vs-effect timing race | **Structural** (the gate), then **narrow** (the regression fix) | Yes — declined to extract a shared reveal-gate primitive or sweep for the same timing pattern elsewhere |
| **08-23** | **PR #2773** | **Reopening a heavily-used pane: 155 backfill events → ~300 overlapping RPC calls → 7.6s stall, rows appear/vanish repeatedly** | Two frontend singletons (`dispatch-source.ts`, `subagent-source.ts`) each fired an uncoalesced refresh on *every single event* | **Narrow, explicitly named as option 1 of 4** — shared 100ms/1s debounce | **Yes, explicitly, twice-named** — "thread the `live`/replay flag to the wire" and "in-flight RPC de-dup" both deferred as "larger, separate" work |
| **08-24** | **PR #2781** | **Owner reports the exact same symptom again, on a build that already had both prior fixes** — "docked items still show, then disappear" | Root cause is explicitly two-part: (1) the debounce coalesces request *volume*, not visual *settlement* — the 2-3 remaining refreshes are still genuinely different real snapshots; (2) the BrainSpinner's readiness gate had **zero** awareness of the dock's data sources at all, ever, by original design | **Structural — and it's the first incident in this whole family where the previously-deferred fix (from 08-23, one day earlier) was actually picked back up rather than left to rot.** New persisted `subagent:backfill_status` event; BrainSpinner now gates on it | Partially — a narrower slice of "thread replay-status to the wire" shipped (scoped to gating the spinner specifically); the more general version (tagging every `subagent:spawned`/`completed` broadcast itself as replay-vs-live) still does not exist |

**Tally of "we should really fix this properly later" notes across this
timeline: 8 distinct, named, still-open architectural deferrals.** Only one
(08-24, PR #2781) was picked back up promptly (the next day) rather than left
sitting. This 1-in-9 follow-through rate is itself worth noting as a process
pattern, separate from the technical root cause below.

**Confirmed hot-spot files, by commit count** (not just doc cross-references
— actual `git log` counts):

| File | Commits | |
|---|---|---|
| `agentmux-srv/src/backend/subagent_watcher.rs` (+ later module split) | **41** | The single hottest file in this entire bug family |
| `frontend/app/view/agent/components/ActivityDock.tsx` | **~14** | Every dock-visible symptom touches this one file |
| `frontend/app/view/agent/activity/subagent-source.ts` | 5 | Every commit since creation has been a bug fix, none a clean feature add |
| `frontend/app/view/agent/activity/dispatch-source.ts` | 3 | 2 of 3 commits are fixes; same shape as its sibling |
| `frontend/app/block/block.tsx` (`ready()` gate) | 3 incidents in 3 consecutive days (08-22 to 08-24), same ~10-line region | |

---

## 3. Why this keeps recurring: the current architecture, precisely

### 3.1 No single authoritative "what's running" source exists

Four structurally independent backend subsystems each answer some version of
"is this thing still active," with **no shared data model, no shared event
vocabulary, and no shared consistency guarantees**:

| Subsystem | Owns | Consistency model |
|---|---|---|
| `agentmux-srv/src/backend/subagent_watcher/` | Subagents + dispatches, via fs-watching Claude Code's own transcript files | **Lazy timer**: a dispatch only flips running→completed on the *next* event or read, 60s after quiet — no backend timer fires this on its own. The frontend has to reimplement this exact 60s constant client-side, kept in sync by comment convention only (no shared constant). |
| `agentmux-srv/src/backend/shell_node.rs` | Persistent shell processes | **Registry-status lookup**, entirely separate map/event names/eviction policy from the above |
| `agentmux-srv/src/backend/storage/background_tasks.rs` | Durable (SQLite) background-task rows — the *only* one of the four that survives an srv restart | Explicitly built because the frontend's own signal chain is "entirely ephemeral... silently evicted after one hour" (its own header comment) |
| `agentmux-srv/src/backend/process_tracker/` | Raw OS process count | Genuinely orthogonal concern, not duplicative of the above three |

These are unified **only at the very last mile**, client-side, by
`ActivityDock.tsx`'s three-way array concatenation of `shellActivities()` +
`subagentActivities()` + `toolActivities()`, and separately re-derived a
second time by `attached-task.ts` for an unrelated liveness boolean — that
file's own doc comment explicitly flags this as duplication that *should*
consume the dock's own signal rather than recompute it, and doesn't.

### 3.2 On the frontend, at least 11 independent modules each maintain their own refresh cycle

A full inventory (not just the 2-3 already implicated in recent bugs) found
11 distinct modules under `frontend/app/view/agent/` that independently
subscribe to backend/WPS events for "what's currently running"-adjacent state.
Most are legitimately separate concerns (process count, controller status,
tab-title text) — but a few are genuine, unreconciled duplication of the same
underlying fact:

- **`dispatch-source.ts` vs `subagent-source.ts`**: two independent
  singletons, two separate RPCs (`ListDispatches` vs `ListActive`), two
  separately-debounced timers, answering overlapping questions derived from
  the *same* underlying backend records. During a backfill burst these two
  singletons' snapshots are not guaranteed to agree with each other at any
  given instant — this is the exact, confirmed mechanism behind the 08-23
  storm's flicker.
- **The durable background-task registry vs. the dock's transcript-derived
  background-task rows**: never reconciled at all, not even via the careful
  union-with-earliest-start-wins pattern used elsewhere in this same codebase
  for a structurally identical problem (see 3.3). A background task that
  survives a session restart — the registry's entire reason for existing — is
  provably invisible as an actual dock row; only a side-effect boolean leaks
  through.

### 3.3 The actual root cause: one missing signal, invented around independently, twice

`subagent_watcher`'s own code already computes a `live: bool` distinguishing
historical replay from a genuine new event (`jsonl.rs`) — but that flag is
used **only** to gate an unrelated side effect (eager Haiku naming) and never
reaches the wire. The WebSocket payload for a replayed 2026-07-01 event is
byte-for-byte identical in shape to a payload for something that just
happened.

Two independent teams/PRs then invented two independent, bespoke workarounds
for the identical underlying gap:

- `dispatch-source.ts`/`subagent-source.ts` got an **event-count debounce**
  (08-23) — masks *volume*, not *content correctness*. This is precisely why
  it did not fully fix the flicker (08-24's retro is explicit about this).
- `useShellNodeStream.ts` got a **bespoke synchronous status-correction RPC**
  plus an ad hoc `reallyResolved` race-guard Set (08-22) — a completely
  different fix, for the same replay-vs-live ambiguity, just for shells
  instead of subagents.

This is a strong, specific structural signal: **two teams independently
discovered the same missing primitive and each built their own local patch
around it**, rather than either one adding the one thing that would have
helped both — a `live`/`replay` field on the relevant WPS event payloads. The
08-24 fix (PR #2781) is the first crack at this, but scoped narrowly to a
single new `subagent:backfill_status` event used only to gate the
BrainSpinner — not the general case of tagging every individual
`subagent:spawned`/`completed` broadcast.

### 3.4 This is not "the whole design is wrong" — it's specific and bounded

Worth being precise here, since "architecture rethink" can be read as
"rewrite everything." It shouldn't be. The *domain* split — shells, subagents,
and promoted tool calls are genuinely different backend concepts with
different lifecycles — is a legitimate distinction, not accidental
duplication. Unifying them only at the final render layer via small, pure,
independently-tested adapter functions (`shell-adapter.ts`,
`subagent-adapter.ts`, `tool-adapter.ts` — each has its own test file) is a
sound shape, and collapsing all three into one unified backend model would be
a large, likely premature undertaking nobody is asking for.

The actual leverage is narrow: **add the missing replay-vs-live signal once,
generically, at the wire level**, and reconcile the two genuinely-unreconciled
duplications named in 3.2. That is real, scoped, understandable work — not a
rewrite.

---

## 4. The broader "modularization" question: is the agent pane itself too big/tangled?

Yes, by the codebase's *own* prior, documented standards — this isn't a new
external judgment being imposed:

### 4.1 `agent-view.tsx` broke its own documented budget by 8.3x

`docs/specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md` set a hard cap of
**≤300 lines** for this file. A verification pass the same week measured it at
302 ("2 lines over, not urgent"). **Today it is 2,496 lines**, with **216
commits** touching it historically — the single most-churned file in the
entire pane module. No later doc revisits or re-authorizes this regrowth; it
simply happened, uncontested, over four months.

It is not alone. `docs/analysis/ANALYSIS_LARGE_FILE_MODULARIZATION_CANDIDATES_2026_05_28.md`
flagged `AgentLaunchModal.tsx` (1,020 lines then) and `agent-view.tsx` (958
lines then, already 3x its own budget) as carve candidates in May, alongside
six Rust files that *did* get modularized the same day (e.g. `store.rs`:
6,226 → 3,306 lines). The TypeScript side of that plan was never executed —
`AgentLaunchModal.tsx` is 900 lines today (roughly flat), `agent-view.tsx` is
2.6x larger than when it was flagged.

Other files currently over 500 lines and mixing concerns:
`useAgentCommands.ts` (1,644), `useAgentControllerStatus.ts` (1,265),
`AgentFooter.tsx`/the composer (1,053 — originally sized small per the April
2026 typing-lag investigation's own "3 principles"), `AgentDocumentVirtualList.tsx`
(1,049), `AgentPicker.tsx` (1,022), `PreLaunchAuthPanel.tsx` (902),
`auth-state.ts` (878).

### 4.2 The team already, explicitly, self-diagnosed scope creep in this exact module

`docs/analysis/agent-pane-rich-features-structure-2026-04-13.md`'s own
author, investigating an unrelated bug, wrote: *"I added seven features the
user didn't ask for... The right longer-term move is probably to remove all
four banners and the two panels entirely... and re-introduce them one by one
only when the user actually asks."* This is a direct, contemporaneous,
first-party admission of the exact pattern you're describing — not a new
outside observation.

### 4.3 The codebase does have a real, disciplined, audited state-management architecture — it's just not what the Activity Dock uses

This is the single most important structural finding for the "do we need an
architecture rethink" question, and the answer is nuanced: **no, you already
built one — a specific new subsystem was built outside it.**

`docs/specs/frontend-reducer-conventions-2026-05-03.md` establishes an
explicit, principled pattern (modeled on the Rust backend's own
`agentmux-srv/src/reducer.rs`): pure `update(state, command) -> {state,
events}` slices, mandatory "negative events" for every suppressed/dropped
command (specifically for audit-ability — exactly the kind of guarantee that
would surface "this refresh was suppressed because a burst was in flight"
instead of silently racing), an echo-loop guard for slices mirroring backend
state, required tests. A companion audit
(`docs/analysis/AGENT_PANE_REDUCER_AUDIT_2026_05_12.md`) confirmed, as of
2026-05-12, that **100% of the agent pane's rendering-relevant state was
already reducer-routed** — a deliberate, verified architecture.

`subagent-source.ts` and `dispatch-source.ts` were built **in June 2026** —
after that audit — to solve a genuinely different problem the reducer
conventions never targeted (a cross-pane shared read cache, since every open
pane's dock needs the same list and a per-pane reducer slot would mean N
redundant polls). That's a legitimate reason not to use the *per-pane-keyed*
slot pattern. But the team then built the shared-singleton case with **none**
of the reducer discipline's other guarantees — no negative/suppressed events,
no audit trail, no invariant enforcement, just a bare `try {} catch {
/* silently ignore */ }` around each refresh. This gap is directly named as
root cause in the 08-24 retro itself: *"there is no principled way to
distinguish 'still converging, don't render this yet' from 'this is the real,
final state' — it can only ever guess via timing."* That is exactly the
invariant the reducer pattern's suppressed-event rule exists to provide.

**14 other files repo-wide** use the same bare-singleton-signal pattern
legitimately for genuinely global, non-keyed state (`global.ts`,
`flash-notifications.ts`, `tab-reveal.ts`, etc.) — so the pattern itself isn't
illegitimate. It's specifically wrong for *this* case, because Activity Dock
data renders a keyed list whose membership visibly changes between snapshots,
which is exactly the scenario the reducer pattern's settlement/audit
guarantees were designed for.

### 4.4 Confirming this is the current hot zone, independent of the docs

**54.5% of all frontend commits since 2026-06-01 (399 of 732) touched
`frontend/app/view/agent/`.** Four distinct flicker-on-load bugs shipped in
this one subsystem within a single 48-hour window (Aug 22-24), each retro
explicitly noting "not the same bug as the last one, a different-but-adjacent
race in the same neighborhood." This is the clearest available signal that
the *architecture* of this one subdirectory — not any individual bug — is the
actual limiting factor.

---

## 5. Recommendations (not implemented — for discussion)

**Tier 0 — do regardless of anything else, costs almost nothing:**
- Cut a release now (§1). Get the already-merged fixes into an actual
  distributed build before investing further effort chasing what might
  already be fixed.

**Tier 1 — narrow, scoped, directly closes the confirmed root cause (§3.3):**
- Add a `live`/`replay` boolean to the WPS payload for
  `subagent:spawned`/`completed`/`activity` broadcasts (the data already
  exists server-side in `jsonl.rs`'s `live` parameter — it just needs to
  reach the wire). Let `dispatch-source.ts`/`subagent-source.ts` skip
  per-event refresh entirely for replay-tagged events, doing exactly one
  fetch once backfill completes — removing the need for the debounce
  heuristic, the `DOCK_SETTLE_BUFFER_MS` guess, and the shell-specific
  synchronous correction RPC as three independent approximations of the same
  missing fact.
- Reconcile the durable background-task registry with the dock's own
  transcript-derived rows (§3.2), using the same union-with-earliest-wins
  pattern already proven for `attachedTask`/`registryAttachedTaskSince`
  elsewhere in this exact codebase — not a new pattern, just applying an
  existing, working one to a second unreconciled case.

**Tier 2 — real but smaller-scoped modularization, not a rewrite:**
- Extract `agent-view.tsx`'s absorbed responsibilities (drag-overlay wiring,
  context-menu wiring, tab-reveal gating) back toward dedicated hooks,
  revisiting the already-written (and already-once-verified-then-abandoned)
  `SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md` plan rather than starting
  fresh.
- Consider whether `activity/`'s two near-identical singleton sources
  (`dispatch-source.ts`/`subagent-source.ts`) should be factored into one
  shared `createPolledEventSource(service, method, events, merge)` helper —
  their own doc comments already admit they're hand-kept-in-sync copies of
  each other; the `subagent:abandoned` wiring bug (PR #2676) already had to
  be fixed twice because of this, once per file.

**Tier 3 — process, not code:**
- Of the 8 "we should fix this properly later" notes accumulated across this
  incident family, only 1 was ever picked back up (and only the next day).
  Worth deciding, as a team norm, whether a deferred-fix note should get a
  tracked follow-up item rather than living only inside a retro doc nobody
  revisits.

**Explicitly not recommended:** a full rearchitecture of the backend
subagent/shell/background-task subsystems into one unified model. The domain
split is legitimate (§3.4); the actual gap is one missing signal plus two
specific unreconciled duplications, not the overall shape.

---

## 6. What this report didn't cover

- The 08-06 phantom-rows incident's fix status was never confirmed as closed
  in any doc found — worth a direct check before assuming it's resolved.
- `stream-parser.ts`'s "no separator between consecutive thinking/text
  deltas" gap, named as Suspect A in the original 2026-05-10 architecture doc,
  is still unaddressed in the code today (verified directly,
  `stream-parser.ts:396-438`) — unrelated to the dock-flicker family, but a
  second confirmed instance of a named, scoped, never-picked-up fix in this
  same module.
- Whether `BACKFILL_MAX_FILES` (200, tuned against the 07-17 OOM crash) is
  still the right cap now that the *consumption*-side cost (this report's
  focus) is better understood was flagged as an open question in the 08-23
  report and not re-examined here.
- No live reproduction of the current symptom was performed as part of this
  report — findings are from code/doc archaeology, not a fresh trace. If the
  flicker persists on a build that actually includes PR #2781, a fresh live
  trace (the same method `REPORT_AGENT_PANE_REOPEN_SUBAGENT_STORM_2026_08_23.md`
  used) would be the right next step to confirm whether it's a genuinely new,
  fifth incident.
