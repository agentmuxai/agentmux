# SPEC — Docs cleanup audit: what's stale, duplicated, or mis-shelved

**Date:** 2026-08-22
**Type:** Analysis + proposal (no files moved yet — this is the plan, not the execution)
**Status:** proposed — nothing has shipped; see §5 for a phased execution plan
**Scope:** `docs/specs/`, top-level `specs/`, `docs/retro/`, `docs/status/`,
`docs/analysis/`, `docs/reports/`, `CLAUDE.md`'s own claims about these
directories. No code changes anywhere — this is documentation hygiene only.

## Why this exists

Asked to do a cleanup pass on the repo. This project already has a real,
documented docs-lifecycle convention
(`docs/specs/README.md`, `SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md`) — a
closed `Status:` enum (`draft | proposed | active | implemented | living |
historical | superseded`) and a stated directory flow
(`docs/specs/` → `specs/` → `specs/archive/`). The problem isn't missing
process, it's that the process isn't being followed at scale, and the volume
involved (see §1) means nobody can just eyeball their way to a fix. This spec
quantifies the actual state and proposes a scoped, phased way to close the
gap without a hand-edit-everything effort that would take longer than it's
worth.

## 1. Scale (measured, not estimated)

| Directory | File count |
|---|---|
| `docs/specs/` (top level) | **686** |
| `docs/specs/archive/` | 26 |
| `docs/specs/evidence/` | 3 |
| top-level `specs/` (top level) | **102** |
| top-level `specs/archive/` | **33** — missed in the first pass of this audit, caught by review; see §2.3 |
| `docs/retro/` | 158 |
| `docs/analysis/` | 132 |
| `docs/reports/` | 40 |
| `docs/status/` | 13 |
| `.changesets/` (pending) | 18 |

Total across just the spec/retro/analysis/report/status families: **~1,211
files.** A full manual audit of all of them is not a reasonable ask of any
single pass, human or agent — §5 scopes what's actually worth doing now vs.
what needs its own tooling.

## 2. Findings

### 2.1 Status-header rot is real and measurable, not just theoretical

The lifecycle spec itself already warned about this ("found shipped features
still marked 'no code yet' 48 hours after landing"). Spot-checked 15
`Draft`/`Proposed`-status specs spread across the project's timeline
(2026-04, 2026-06/07, 2026-08): **6 of 15 (40%) describe features that have
already shipped**, confirmed against actual code:

- `SPEC_BROWSER_AND_EDITOR_PANES_2026_04_16.md` — editor/browser panes are
  live, shipped widgets today.
- `SPEC_AGENT_PANE_ZONE_ORDER_WORKED_FOOTER_2026_04_24.md` — the "Worked"
  footer exists verbatim in `AgentFooter.tsx`.
- `SPEC_AGENT_TOOL_CALL_TONES_2026_06_05.md` —
  `frontend/app/notification/sound/tool-tones-player.ts` exists and is the
  same system this session's own DnD/watchdog PRs touched adjacent settings
  for.
- `SPEC_AGENT_SESSION_COST_TOTALS_2026_07_02.md` — cumulative cost/token
  totals are implemented in `AgentComposerStrip.tsx`.
- `SPEC_AGENT_STARTUP_SEQUENCE_2026_04_16.md` — core startup flow long since
  built.
- `SPEC_AGENT_VIEW_SCSS_SPLIT_2026_04_24.md` — **correction, made while
  executing this spec's own Phase 1**: originally characterized here as
  "partially done... half-done" based on `agent-view.scss` still being 387
  lines. A closer read (the file is now almost entirely `@use` imports of 28
  split files matching this spec's target names, with the remaining ~350
  lines being legitimate root-layout rules, not unmigrated component
  styles) found it's actually essentially complete, not half-done. Another
  instance of this spec's own shallow-verification problem (§2.2) — line
  count alone wasn't enough evidence; fixed once the file was actually read.

I found this exact same rot on a smaller scale earlier this session:
`SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md` still said "Status:
Draft... not yet implemented" three days after it shipped in #2751 — I
corrected that one directly (§5.1 covers doing the equivalent at scale).

### 2.2 Retracted: "duplicate specs never marked as such" — the original claim was wrong

**This section originally claimed five pairs of same-subsystem, close-dated
specs were undocumented duplicates.** A PR review (Codex, on the PR carrying
this spec) caught that the bundles pair's actual superseded-by targets were
different documents entirely — which prompted re-checking all five pairs
against their own file content instead of just matching on topic + date
proximity (the method originally used, and the actual mistake). Result: **all
five were wrong**:

- `SPEC_TRUST_CENTER_RENAME_2026_07_02.md` already states *"Superseded by
  `SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md`"* in its own archive
  banner. Already correctly cross-referenced.
- `SPEC_OAUTH_IN_IDENTITY_BUNDLES_2026_05_13.md` already states *"Superseded
  by `SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md`"*. Already correctly
  cross-referenced.
- `SPEC_SHARED_BUNDLES_AND_DEFINITIONS_2026_05_19.md` is superseded by
  `SPEC_GLOBAL_IDENTITY_MEMORY_DRONE_2026_06_24.md` (not by
  `SPEC_BUNDLE_MANAGEMENT_2026_05_22.md`, which is itself superseded by
  `specs/archive/SPEC_TRUST_CENTER_2026_06_15.md`) — the Codex-caught error.
  Sharing a subsystem and nearby dates doesn't make two docs successive
  versions of each other.
- `SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md` and
  `SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md` are both explicitly sequential
  phases of the *same* governing proposal
  (`specs/PROPOSAL_COMPOSABLE_AGENT_MODEL_2026_06_30.md`) — the second
  builds on the first (v2 of a container model, referencing v1's shipped
  primitives), not a replacement of it. Not a duplicate at all.
- `ARCHITECTURE_ARMORY_2026_07_20.md` and
  `ARCHITECTURE_ARMORY_FOUNDATION_CONSOLIDATION_2026_08_19.md`: the latter's
  own header lists the former under **Related**, not as something it
  replaces — one is a living reference doc, the other an explicit
  incremental "north star" that cites the reference doc as background. Not
  a duplicate.

**Net result: I have zero verified examples of this problem right now.**
The lifecycle convention's `superseded` status + mandatory
`**Superseded-by:**` pointer may still be worth double-checking at scale
(§5.5's deferred full audit would surface real cases if any exist), but this
spec no longer claims to have found any, and the Phase 1 action item to add
pointers for these five pairs (originally in §5.1) is withdrawn.

### 2.3 Corrected: TWO archive directories are both active, not one "wrong" one

**Original claim (wrong):** archived specs only land in `docs/specs/archive/`
(26 files) despite the README saying `specs/archive/`, implying the README
was simply never followed. **Corrected after Codex flagged this**: a
top-level `specs/archive/` also exists, and holds **33 files** — more than
`docs/specs/archive/`'s 26. Both are real, both are in active use. This
isn't "the README is wrong," it's two archive locations I hadn't fully
checked.

A closer look at both suggests (not yet confirmed) a two-tier explanation
rather than a conflict: files across *both* directories carry the same
`"> **Archived 2026-07-12.** ... Consolidated tracking: issue #2024."` banner
— e.g. `docs/specs/archive/SPEC_BUNDLE_MANAGEMENT_2026_05_22.md` and
`specs/archive/SPEC_FORGE_IDENTITY_AGENT_INSTANCES_IMPL_2026_04_20.md` both
have it. That reads like a single 2026-07-12 consolidation event (issue
#2024) added archived-banners to specs wherever they already physically
lived, rather than moving everything into one canonical folder — plausibly
`specs/archive/` holds specs that had already been promoted to the approved
`specs/` tier before being archived (matching the README's stated flow
exactly), while `docs/specs/archive/` holds specs archived directly from
draft stage, without ever going through the `specs/` promotion step. **This
is a hypothesis, not a confirmed finding** — it would need reading issue
#2024 itself or more of the consolidation's own history to confirm. Flagged
as Open Question 1, replacing the original (wrong) "pick one" framing.

### 2.4 Withdrawn: the `CLAUDE.md` / `specs/` AGENTBUS claim

**Original claim:** `CLAUDE.md`'s line *"Old specs in `specs/` use
`AGENTBUS_*`... those are historical documents"* is stale because only 5 of
103 files in `specs/` actually mention `AGENTBUS`. **Withdrawn** — Codex's
review correctly pushed back: the sentence says *"old specs"* (a subset),
not *"all specs"*; counting total `AGENTBUS` mentions across the whole
directory doesn't establish that the sentence is inaccurate about the subset
it's actually describing. My original reasoning conflated "most files in
`specs/` aren't AGENTBUS-related" (true, and unremarkable — a directory can
contain both current and superseded material) with "the sentence claims all
files are AGENTBUS-related" (never actually claimed). No fix recommended
here; the corresponding Phase 2 action item (§4, originally item 2) is
withdrawn along with it.

### 2.5 `docs/status/` has apparently-still-open items with no resolution marker

Unlike `docs/retro/` (explicitly historical/append-only by design) or
`docs/specs/`, `docs/status/` reads as "investigation in progress" — and at
least two files show that ambiguity concretely:
`STATUS_SRV_SECTION_HANDLE_LEAK_2026_08_08.md` got a "LIVE_RECURRENCE"
follow-up 11 days later (so the original wasn't actually resolved, or
regressed), and `STATUS_IDENTITY_ISOLATION_GATE_NOT_ENFORCING_2026_08_20.md` /
`STATUS_CROSS_CHANNEL_AGENT_OPEN_FULL_APP_FREEZE_2026_08_22.md` read as open
bugs with no resolved marker at all (as of this writing). With only 13 files
total, this family is small enough to actually hand-verify (see §5.3),
unlike the 686/103-file spec directories.

### 2.6 Not everything here is actually a problem

- `.changesets/`: 18 pending, all timestamped the same day — a normal,
  healthy backlog awaiting the next `task release`, not a cleanup target.
- `docs/retro/` (158) and `docs/analysis/` (132): these are explicitly
  historical-record documents by design (retros/investigations), not living
  specs with a Status field expected to track current reality. Volume alone
  isn't a problem here — flagged as out of scope for status-correction work,
  though §5.4 proposes one narrow check.

## 3. What this spec does NOT try to solve

- **A full manual status-audit of all ~847 spec files** (`docs/specs/` +
  `docs/specs/archive/` + top-level `specs/` + `specs/archive/`) — at a ~40%
  stale rate on the (small, `docs/specs/`-only) sample, that's plausibly
  300+ files needing a corrected Status line. That's not a "cleanup pass,"
  it's a multi-day project of its own. §5 scopes a bounded, high-value subset
  instead.
- **Auto-deleting anything.** Every action below is move-to-archive or
  edit-a-status-line, never delete — specs are historical record even once
  superseded/implemented.
- **Re-litigating which features described in still-open Draft specs should
  actually be built.** Out of scope; this is about the paperwork accurately
  reflecting reality, not product prioritization.

## 4. Proposed changes to the convention itself (before any bulk pass)

Both original items in this section (§2.3's archive-location "fix" and
§2.4's `CLAUDE.md` fix) are **withdrawn** — see the corrections in §2.3/§2.4
above. Nothing here is confidently recommended yet; §2.3's two-tier
hypothesis would need confirming (reading issue #2024) before any
convention change is proposed.

## 5. Phased execution plan

### 5.1 Phase 1 — targeted status-header correction (bounded, do now)
Re-verify and correct the Status line on the exact 6 stale specs identified
in §2.1 (concrete, already-confirmed list, double-checked against current
headers before finalizing this spec — no further investigation needed).
**6 files** — the 5 duplicate/superseded-pair file edits originally proposed
here are withdrawn along with §2.2's retracted claim.

### 5.2 Phase 2 — withdrawn
Both documentation-bug fixes this phase covered (§4's original items 1 and
2) are withdrawn per §2.3/§2.4's corrections. Nothing to execute here unless
§2.3's hypothesis is confirmed and turns into an actual recommendation.

### 5.3 Phase 3 — `docs/status/` resolution sweep (small, bounded)
Only 13 files. For each: check whether the underlying issue is actually
fixed in current code (git log + code inspection, same method used in §2.1's
spot-check) and either add a clear "RESOLVED — see PR #N" line or leave it
explicitly open. This is small enough to do as a single pass, unlike the
spec directories.

> **✅ EXECUTED 2026-08-29.** **15 files, not 13** — two were added after
> this spec was written (`STATUS_COMPOSER_STRIP_ZONE_BALANCE_HANDOFF_2026_08_25`,
> `STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23`). Outcome:
>
> | Outcome | Count | Files |
> |---|---|---|
> | Newly marked **RESOLVED** with PR citations | 4 | srv section handle leak (08-08) + its live recurrence (08-19) → #2666/#2673; cross-channel stale `session_id` (08-20) → #2755/#2693/#2776; stale-resume repro + fix plan (08-23) → §6.1→#2755, §6.2→#2776 |
> | Newly marked **SUPERSEDED** | 1 | attached-task ladder (08-09) → the 08-15 doc |
> | **Partially** resolved, left open | 1 | attached-task axis (08-15) — #2491/#2492 closed via #2589/#2590/#2681/#2683, but its architectural proposal is unbuilt |
> | **Corrected** stale content, still open | 1 | messenger integrations (07-07) — §4's per-platform table said "Not started" for Slack/Telegram/WhatsApp, all shipped (#2026/#2022/#2028) |
> | Marked **HISTORICAL** | 1 | issue/discussion cleanup (07-24) — its one open item, PR #2291, merged 2026-07-25 |
> | **Left explicitly open** with a dated staleness note | 3 | app-freeze (08-22), lifecycle+crash architecture (07-16), lifecycle/OOM refactor (06-30) |
> | Already RESOLVED and already citing PRs — untouched | 4 | fs-watch sweep leak, composer strip, identity isolation gate, pagefile growth |
>
> **Two findings worth carrying forward:**
> 1. **Docs went stale *internally*, not just against code.** The messenger
>    doc contradicted itself — its §1 table and update block recorded four
>    shipped bridges while §4 still read "Not started" for three of them.
>    A sweep that only diffs docs against *code* would have missed this.
> 2. **"RESOLVED" was under-propagated across linked docs.** The 08-19
>    recurrence doc opened by calling the 08-08 doc "still open, unpatched"
>    long after #2666 fixed it. When a fix lands, the docs that *cite* the
>    broken one need updating too, not just the doc that owns the bug.
>
> Deliberately **not** done: per-item verification of the two in-flight
> architecture programs (lifecycle/OOM, lifecycle+crash). A docs-status
> sweep is the wrong instrument for auditing an active program, and
> guessing is precisely this spec's §2.2 failure mode. Both carry an
> explicit note saying what was and was not checked.

### 5.4 Phase 4 (optional, lower priority) — one check on docs/analysis
Spot-check whether any `docs/analysis/` findings docs describe a bug that
has since been fixed without a resolution note (mirroring §5.3's method) —
only worth doing if Phase 3 turns up a pattern suggesting this family has the
same problem; not committing to it here without evidence.

> **✅ TRIGGER MET, SPOT-CHECK EXECUTED 2026-08-29.** Phase 3's finding that
> "RESOLVED" was under-propagated across *linked* docs is exactly the
> evidence this phase was gated on, so it was run — as the spot-check this
> section specifies, **not** a sweep. `docs/analysis/` holds **132 files**;
> a full pass belongs with §5.5's deferred effort, not here.
>
> **Sample: 8 files**, chosen for relevance rather than at random — the
> scroll-oscillation chain (adjacent to Phase 3's live threads) plus four
> unrelated bug analyses picked to test whether the pattern generalises
> beyond the cluster already known to be active.
>
> **4 hits in 8 — the pattern does generalise:**
>
> | File | Problem | Fixed by |
> |---|---|---|
> | `ANALYSIS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_17` | §6 still says its key question "needs a live repro this agent cannot drive." That repro ran twice since; no forward pointer existed. | Not fixed — but the blocker moved. Repros in the 08-21 and 08-22 findings docs; root cause still open (#2648, #2718) |
> | `FINDINGS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_21` | Dead-ends at 8 events from one pane; no pointer to the ~100× larger dataset that followed. | Superseded in scale by the 08-22 findings doc |
> | `ANALYSIS_BROWSER_PANE_BLACK_FREEZE_MACOS_2026_06_24` | Read as an open bug. Carried **no resolution note at all** — despite a retro on the fix sitting in the same directory, unlinked. | **#1769** (same day) + **#1778** |
> | `ANALYSIS_MUXBUS_WINDOWS_SIGNIN_URL_TRUNCATION_2026_07_03` | Status cited an in-flight **branch**, not a merged PR — so it read as unshipped long after it landed. | **#1938**, verified live in `util.rs`'s `open_browser` |
>
> All four were corrected in place. The other 4 sampled files were fine.
>
> **What this says about §5.5.** A 50% hit rate on a small relevance-weighted
> sample is not a clean estimate for all 132 files, and shouldn't be quoted
> as one. But it does confirm `docs/analysis/` has the same disease as
> `docs/specs/`, and adds a failure mode the §2.1 sampling never looked for:
> **the doc that owns a bug and the doc that records its fix can both be
> correct while nothing links them.** Any batch process for §5.5 needs to
> check cross-document linkage, not just each file against code — three of
> the four hits above were invisible to a file-vs-code check.

### 5.5 Explicitly deferred — the full ~847-file spec audit
Given the ~40% stale rate found on a 15-file `docs/specs/`-only sample, a
defensible estimate is several hundred specs need a corrected Status line
across all four directories combined. Doing this by hand, spec-by-spec,
doesn't scale, **and this spec's own track record in §2.2/§2.3/§2.4 is a
concrete warning against doing it via quick pattern-matching**: shallow
topic/date matching produced five confidently-stated but false claims in
this very document, caught only because a reviewer checked the actual file
content. Any batch process for the full audit must verify against real file
content (the way §2.1's spot-check did, and the way §2.2's re-check should
have from the start), not proximity heuristics. Recommend (not designed in
full here, flagged for its own follow-up): one agent per spec (or small
batch) reads the spec, checks whether its described feature/file exists in
current code AND whether the spec's own text already contains a
Superseded-by/Archived banner, and proposes a corrected Status line for
human review — never auto-editing unattended. Flagged as a candidate if the
user wants to commit to that scale of effort, not started here.

## Open questions

1. **Archive-location convention (§2.3)** — confirm the two-tier hypothesis
   (promoted-then-archived specs in `specs/archive/` vs. archived-from-draft
   specs in `docs/specs/archive/`) before proposing any convention change —
   likely needs reading the issue #2024 consolidation history itself, not
   more file sampling.
2. **Appetite for §5.5's full audit** — this spec deliberately scopes Phase
   1 to the 6 files already identified and double-checked in §2.1, not a
   commitment to fix an estimated 300+ stale specs across ~847 files.
   Confirm whether that larger effort is wanted (and at what scale/budget)
   before committing to it — and if so, budget real per-file verification
   time, not a repeat of this spec's own §2.2 mistake.

## References

- `docs/specs/README.md` — the stated directory-flow convention; §2.3's
  corrected finding means this needs re-reading against the two-tier
  hypothesis, not treated as simply "wrong."
- `docs/specs/SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md` — origin of the
  closed Status enum and its own warning about status rot.
- `docs/specs/SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md` — corrected
  earlier this session as a concrete single-file example of exactly the
  status-rot pattern §2.1 quantifies at scale.
- PR review comments on the PR carrying this spec (Codex) — caught the
  false claims in §2.2/§2.3/§2.4; retained here as the actual source of
  those corrections rather than silently rewritten away.
