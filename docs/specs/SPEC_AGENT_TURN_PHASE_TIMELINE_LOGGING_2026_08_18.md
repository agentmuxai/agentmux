# SPEC: Agent turn-phase timeline — unified, replayable phase-history logging + `muxlog phases`

**Date:** 2026-08-18
**Author:** AgentA
**Status:** Implemented

---

## TL;DR

Give agents (and humans) a reliable way to reconstruct, after the fact, the
full turn-phase timeline for any agent pane — every `Idle`/`Submitting`/
`Streaming`("Working…")/`Interrupting`/`Done` transition, correlated across
the frontend's `[wave-turn]` log and the backend's `[health]` log, plus the
specific diagnostic events already identified as failure points (stray
`StreamFlushObserved` re-promotions, `StreamWatchdogTick` fires/misses,
`document.visibilitychange` catch-ups) — via a new `muxlog phases` recipe.
The goal: an agent asking "why did my pane show Working the whole time"
should get a direct, chronologically-merged answer from one command, not
the multi-hour manual log archaeology the prior investigation into exactly
this question required.

---

## Why now

- **Real, expensive precedent.** `docs/reports/REPORT_AGENTA_STUCK_WORKING_INVESTIGATION_2026_08_14.md`
  root-caused a stuck-"Working" pane, but getting there required: manually
  locating the right instance's log directory (the default picker returned
  a wrong instance silently), cross-referencing TWO separate log files
  (host + sidecar) by hand, and inferring "the watchdog probably wasn't
  ticking" from the *absence* of an expected line rather than being able to
  confirm it directly (§4: *"If that interval were actually ticking for
  this pane, we would see it in the log — and we don't... The only
  explanation consistent with total silence... is that `StreamWatchdogTick`
  was never dispatched."*). That's a lot of inference for a question that
  should be a direct lookup.
- **This exact gap resurfaced today (2026-08-18).** Asked "why was I stuck
  Working the whole time," the only honest answer was "two plausible
  mechanisms, here's the evidence for each, I'd need to pull this session's
  own logs to say which one for sure" — the same manual process the Aug 14
  report describes, not yet made routine.
- **The underlying telemetry already exists but isn't correlated or
  surfaced as a first-class query.** `docs/reports/REPORT_WORKING_STATE_TELEMETRY_AUDIT_2026_07_27.md`
  catalogued 9 false-positive "Working" paths and prescribed logging; the
  `[wave-turn]` (frontend) and `[health] turn_active flip` (backend) lines
  it called for are shipped and confirmed present on `main`. What's missing
  is turning two raw, separately-located NDJSON streams into one readable
  timeline.
- **This is exactly the kind of self-audit agents should be able to do
  routinely**, not just during a dedicated incident investigation — after
  a long or unusual turn, "let me check how my own pane actually behaved"
  should be a cheap, zero-setup command.

---

## Current state of the code

- **Frontend transitions:** `frontend/app/store/agent-pane-state-store.ts`
  emits `console.info` on every `turnPhase.kind` change, tagged
  `[wave-turn]`. This lands in the **host** log (`muxlog`'s `fe` target is
  exactly "the host log, pre-filtered to `[fe]` lines").
- **Backend health flips:** `agentmux-srv/src/backend/blockcontroller/health.rs`
  emits `[health] turn_active flip` telemetry into the **sidecar (srv)**
  log — a different file, in some cases a different log root entirely
  (`~/.agentmux/dev/<branch>/logs/` vs `~/.agentmux/logs/`).
- **No existing tool merges these two by timestamp.** `muxlog`'s own doc
  (`docs/MUXLOG.md`) lists a combined `all` view as an explicit **roadmap
  item**, not yet built. The three existing correlated recipes
  (`muxlog swarm`, `muxlog auth`, `muxlog bridge` —
  `agentmux-srv/src/backend/shellintegration/muxlog.mjs:381-434`) are each
  a single-log-file `--grep` preset; `auth` spans several *srv modules* but
  still only one *file*. None cross host+srv.
- **Watchdog logging is edge-triggered only.** `agent-pane-state-store.ts`'s
  `slot.stuckLogged` gate means a *healthy* tick produces zero log output —
  "is the watchdog alive right now" can only be inferred from the absence
  of a stuck/recovered line over a known window, which is fragile and is
  exactly what made the Aug 14 investigation slow.
- **Self-identification is already solved for the common case.**
  `mcp__agentmux__WhoAmI` returns an agent's own `block_id` instantly, and
  every agent's shell env already carries `AGENTMUX_BLOCKID` (confirmed
  directly in this session's own `env` output). The Aug 14 report's
  difficulty resolving "which pane is AgentA" was a **third-party**
  lookup problem (one agent identifying *another* agent's block by
  elimination, since no name→block_id API exists) — not a self-lookup
  problem. This spec's core ask (agents reviewing their own history) does
  not need that harder case solved.

---

## Target state

### 1. New muxlog recipe: `muxlog phases [<block-id>]`

- **No argument → the caller's own pane.** Resolve via `$AGENTMUX_BLOCKID`
  (already present in every agent's shell env) so the common case —
  "show me my own phase history" — needs zero setup, matching how
  `WhoAmI` already makes self-identification a non-problem.
- **Explicit `<block-id>` → that pane**, for a human or another agent
  investigating a specific block once they already have its id (from
  `muxspect list`, a bug report, etc.). Name→block_id resolution is
  explicitly out of scope for v1 (see Non-goals).
- **Merges, chronologically, into one stream:**
  - `[wave-turn]` transition lines (host log)
  - `[health] turn_active flip` lines (srv log)
  - the new explicit watchdog-tick and visibility-change lines (items 2–3
    below)
  scoped to the target block id, sorted by timestamp across both source
  files.
- **Render format**, one line per event:
  ```
  HH:MM:SS  <source>  <from-phase> → <to-phase>   cmd=<trigger>  [toolsActive=N currentTool=X]
  ```
  e.g.
  ```
  13:29:07.328  fe    Idle → Done             cmd=TurnEnd
  13:29:07.307  srv   turn_active: true→false
  13:29:32.191  fe    Done → Streaming        cmd=StreamFlushObserved (stray)  toolsActive=0
  13:30:17.xxx  fe    watchdog: tick (no recovery — 45s idle)
  ```
- **Same standard options every other muxlog target already has**
  (`--since`, `-n`, `--raw`, `-i`) for consistency — no new mental model
  to learn.

### 2. Make the watchdog tick observable, not just its edge effects

Add a `[wave-turn] watchdog: tick` line the periodic `StreamWatchdogTick`
handler already dispatches every 5s
(`frontend/app/view/agent/hooks/useTurnLifecycle.ts:233-239`) — gated to
avoid noise (see Design questions) rather than one line per tick
unconditionally. This directly removes the exact evidentiary gap the Aug
14 report had to reason around ("we'd see a line if it were ticking, we
don't, therefore...") — replacing an inference with a direct fact.

### 3. Log `document.visibilitychange` transitions explicitly

PR #2575's catch-up-tick fix depends on `visibilitychange` firing reliably
even for a throttled/backgrounded window, but nothing currently confirms,
after the fact, that it actually fired for a given incident — only the
resulting (indistinguishable-from-a-regular-tick) `StreamWatchdogTick`
dispatch. Add `[wave-turn] visibility: hidden→visible` /
`visibility: visible→hidden` lines at the point PR #2575's listener fires
(`useTurnLifecycle.ts`'s `onVisibilityChange`).

### 4. Explicitly tag a stray/late `StreamFlushObserved` re-promotion

Today, a legitimate multi-round continuation and a stray/late flush both
log identically as `cmd=StreamFlushObserved` — an investigator has to
reconstruct which one happened from surrounding context by hand (exactly
what the Aug 14 report's §3 did manually, cross-referencing the backend's
own `active:false` timestamp). Since the reducer already knows the prior
phase it's re-promoting *from* (`Done`/`Idle`/`Disconnected` vs. an
in-progress `Streaming` continuation), tag the "stray" case distinctly at
the point of dispatch — e.g. `cmd=StreamFlushObserved (stray)` when the
prior phase was already terminal.

### 5. (Stretch — not required for v1) Name→block_id lookup

Would let `muxlog phases <agent-name>` work directly and remove the
"confirmed by elimination" workaround the Aug 14 report resorted to for a
third-party lookup. Flagged as optional: it needs a new REST surface
(`/api/v1/blocks`/`/api/v1/agents` currently return empty for this), and
the actual ask driving this spec — *"agents can come back later to see how
**they** operated during **their** work"* — is the self-case, which item 1
already covers via `$AGENTMUX_BLOCKID`. Worth a follow-up if third-party
investigation (a human debugging a different agent's report) turns out to
need it often enough to justify the new endpoint.

---

## Non-goals

- **Not fixing the underlying stuck-Working bug class.** The stray-flush
  re-promotion (July 27 report's risk #7) and the still-unconfirmed
  CEF-background-window timer-throttling hypothesis (Aug 14 report §4,
  §6) remain open, tracked separately. This spec is purely about better
  evidence for the *next* investigation — it does not change runtime
  behavior of the state machine itself.
- **Not building a general multi-log-merge engine.** `muxlog`'s `all`
  target (a fully general cross-log combined view) stays a separate
  roadmap item. This spec scopes the merge narrowly to the two known
  sources `phases` actually needs (host `[wave-turn]` + srv `[health]`),
  which is a much smaller, well-defined problem than a general combiner.

---

## Design questions / open items

- **Where does the merge run?** Leaning toward: no new backend surface —
  the Node `muxlog.mjs` core opens both files directly (same approach
  `swarm`/`auth`/`bridge` already use for their single files) and merges
  by parsed timestamp in-process. Matches the existing architecture with
  no new RPC/API needed for v1.
- **Watchdog tick logging cost.** Logging every 5s tick for every open
  pane, across every running instance, in a long dev session could get
  noisy fast. Options: gate behind `--verbose`/a debug level excluded by
  muxlog's default `--level` filter (mirroring how `-a` opts into
  agent-transcript noise), or log only a coarser "still ticking, check
  #N" line every ~12 ticks (~1/min) as a cheap heartbeat instead of every
  5s. Needs a decision before implementation, not before this spec.

---

## Implementation notes (2026-08-18, same day)

Both design questions above were resolved during implementation:

- **Merge location:** confirmed the no-new-backend-surface approach —
  `muxlog.mjs`'s new `phasesTimeline()` opens both files directly and
  merges by parsed `timestamp` string (ISO 8601 sorts correctly
  lexically, no date parsing needed).
- **Watchdog tick cost:** went with the coarser option — one
  `watchdog: tick #N — alive, phase=<kind>` heartbeat every 12th
  `StreamWatchdogTick` dispatch (~60s at the 5s interval), tracked per-pane
  in `agent-pane-state-store.ts`'s `Slot.watchdogTickCount`. Counts
  independently of whether the reducer's own logic finds anything to do —
  it's proof the interval itself fired, which is the specific fact §4 of
  the Aug 14 report had to infer from absence rather than confirm.

**A real instance of exactly the failure mode item 1 warns about was
caught live while manually verifying the recipe.** The first
implementation correlated host→srv by `source` alone (`shared` /
`dev:<branch>` / `channel:<x>`). Testing against synthetic fixtures placed
in the shared log root (`~/.agentmux/logs/`) — deliberately, since that's
where every real release and portable build also logs — showed the bug
immediately: with no `-i` given, the host log resolved to the fixture
(most recently touched), but the srv log resolved to the machine's own
real, currently-running production instance instead of the fixture's
paired srv file, because both innocently share `source: "shared"`. Fixed
by correlating on `(source, version)` together — a real host/srv pair from
the same instance shares both, since both filenames embed the same
version string — falling back to `source`-only, then to the plain
most-recent pick, only if no exact pairing exists. This is exactly the
"which pane is this NDJSON line actually about" class of bug this whole
spec exists to make easier to avoid — worth keeping as a concrete example
for future muxlog recipes that need to correlate more than one log file.

Verified: `tsc --noEmit` clean; `npx vitest run` on the three touched
frontend files (216/216 passing, including new coverage for the
stray-flush tag, the watchdog heartbeat cadence, and both
`visibilitychange` directions); `node --check` on `muxlog.mjs`; and a
manual end-to-end run of `muxlog phases` against synthetic host+srv NDJSON
fixtures covering: correct block-id filtering (excluding another pane's
`[wave-turn]` lines and another block's `[health]` line), correct
chronological interleaving across the two files, the `(stray)` tag,
`--raw`, `-n`, the `$AGENTMUX_BLOCKID` default, and both "no block id
given" / "no matching lines" error paths.

---

## Test plan (for whoever implements)

- Unit test the phases recipe's merge/sort logic against synthetic host +
  srv NDJSON fixtures with interleaved, out-of-order-on-disk timestamps.
- Live-verify against a real pane: reproduce a stray `StreamFlushObserved`
  re-promotion (repro steps from the July 27 / Aug 14 reports) and confirm
  `muxlog phases` shows the full timeline correctly ordered and correctly
  labeled, including the new tick/visibility lines.
- Confirm `muxlog phases` with no arguments, run from inside an agent's own
  tool-spawned subshell, resolves the caller's own block via
  `$AGENTMUX_BLOCKID` with zero manual lookup — this is the core ask this
  spec exists to satisfy.

---

## References

- `docs/reports/REPORT_AGENTA_STUCK_WORKING_INVESTIGATION_2026_08_14.md` —
  the incident that motivates this spec; also the source of the `(stray)`
  StreamFlushObserved distinction and the watchdog-tick-visibility gap.
- `docs/reports/REPORT_WORKING_STATE_TELEMETRY_AUDIT_2026_07_27.md` —
  original telemetry audit; `[wave-turn]`/`[health]` logging traces back
  to this.
- `docs/MUXLOG.md` — existing recipe conventions this spec extends.
- `agentmux-srv/src/backend/shellintegration/muxlog.mjs` — implementation
  home for the new `phases` recipe.
- `frontend/app/store/agent-pane-state-store.ts` — `[wave-turn]` source.
- `frontend/app/view/agent/hooks/useTurnLifecycle.ts` — watchdog interval,
  visibility listener, PR #2575's fixes.
- `agentmux-srv/src/backend/blockcontroller/health.rs` — `[health]` source.
