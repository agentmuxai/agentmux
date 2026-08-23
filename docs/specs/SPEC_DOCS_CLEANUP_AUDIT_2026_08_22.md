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
| top-level `specs/` | **103** |
| `docs/retro/` | 158 |
| `docs/analysis/` | 132 |
| `docs/reports/` | 40 |
| `docs/status/` | 13 |
| `.changesets/` (pending) | 18 |

Total across just the spec/retro/analysis/report/status families: **~1,161
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
- `SPEC_AGENT_VIEW_SCSS_SPLIT_2026_04_24.md` — partially done (a `styles/`
  split directory now exists) but `agent-view.scss` itself is still 387
  lines; the header doesn't reflect the partial state at all, which is worse
  than either extreme (a reader can't tell it's half-done).

I found this exact same rot on a smaller scale earlier this session:
`SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md` still said "Status:
Draft... not yet implemented" three days after it shipped in #2751 — I
corrected that one directly (§5.1 covers doing the equivalent at scale).

### 2.2 Duplicate/superseding specs never marked as such

The lifecycle convention has a `superseded` status with a mandatory
`**Superseded-by:**` pointer specifically to prevent this — it isn't being
used. Found (dates confirm which came first):

- `docs/specs/archive/SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md` and
  `docs/specs/archive/SPEC_TRUST_CENTER_RENAME_2026_07_02.md` — same date,
  near-identical title, both archived with no cross-reference.
- `docs/specs/archive/SPEC_OAUTH_IN_IDENTITY_BUNDLES_2026_05_13.md` →
  `docs/specs/archive/SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md` (9 days
  apart, same topic).
- `docs/specs/archive/SPEC_SHARED_BUNDLES_AND_DEFINITIONS_2026_05_19.md` →
  `docs/specs/archive/SPEC_BUNDLE_MANAGEMENT_2026_05_22.md` (3 days apart).
- `docs/specs/SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md` →
  `docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md` (~6 weeks apart,
  sequential evolution of the same bundle subsystem).
- `docs/specs/ARCHITECTURE_ARMORY_2026_07_20.md` vs.
  `docs/specs/ARCHITECTURE_ARMORY_FOUNDATION_CONSOLIDATION_2026_08_19.md` (1
  month apart, same subsystem).

A reader hitting the older doc in any of these pairs has no way to know a
newer, authoritative version exists — exactly the failure mode `superseded` +
`Superseded-by:` was designed to prevent.

### 2.3 The directory convention itself is inconsistently applied

`docs/specs/README.md` states specs move to `specs/archive/` once
implementation is complete. In practice, archived specs land in
`docs/specs/archive/` instead (26 files there today) — the README describes
a flow that isn't what's actually happening. Either the README is wrong or
the archiving has been happening in the wrong place; **this needs a decision
before any bulk-move cleanup, not a unilateral pick** — see Open Question 1.

### 2.4 `CLAUDE.md`'s own claim about top-level `specs/` is stale

`CLAUDE.md` says: *"Old specs in `specs/` use `AGENTBUS_*` — those are
historical documents describing the predecessor service."* Checked: only
**5 of 103** files in `specs/` actually mention `AGENTBUS`. The other 98 cover
still-relevant, still-referenced areas (Armory, MCP/Skills primitives) —
`CLAUDE.md` itself elsewhere points to `specs/SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md`
as current guidance in the same file that calls the directory "old." This is
a documentation bug in a file every agent reads as ground truth for how to
behave — worth fixing independent of anything else in this spec (see §5.2).

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

- **A full manual status-audit of all 789 spec files** (`docs/specs/` +
  top-level `specs/`) — at a ~40% stale rate on the sample, that's plausibly
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

1. **Reconcile the archive-location mismatch (§2.3).** Recommend: keep
   `docs/specs/archive/` as the real destination (it's where the actual
   archiving has been happening, 26 files' worth of precedent) and fix
   `docs/specs/README.md` to say so, rather than trying to migrate 26
   already-archived files to match a README that was seemingly never
   followed. This is a one-line doc fix, not a file-mover.
2. **Fix `CLAUDE.md`'s stale characterization of `specs/`** (§2.4) — replace
   the "old, AGENTBUS-only" framing with something accurate (e.g. "approved
   specs move here from `docs/specs/` once ready for implementation; most are
   current and actively referenced — only a handful of pre-`muxbus`-rename
   docs use the historical `AGENTBUS_*` naming").

## 5. Phased execution plan

### 5.1 Phase 1 — targeted status-header correction (bounded, do now)
Re-verify and correct the Status line on the exact 6 stale specs identified
in §2.1 (concrete, already-confirmed list — no further investigation needed)
plus the 5 duplicate/superseded pairs from §2.2 (10 files, add
`Superseded-by:` pointers). **11-16 files total, fully specified above** —
small enough to execute directly as a follow-up to this spec, not a new
research phase.

### 5.2 Phase 2 — the two documentation-bug fixes from §4
Both are single-file edits (`docs/specs/README.md`, `CLAUDE.md`). Do
alongside Phase 1.

### 5.3 Phase 3 — `docs/status/` resolution sweep (small, bounded)
Only 13 files. For each: check whether the underlying issue is actually
fixed in current code (git log + code inspection, same method used in §2.1's
spot-check) and either add a clear "RESOLVED — see PR #N" line or leave it
explicitly open. This is small enough to do as a single pass, unlike the
spec directories.

### 5.4 Phase 4 (optional, lower priority) — one check on docs/analysis
Spot-check whether any `docs/analysis/` findings docs describe a bug that
has since been fixed without a resolution note (mirroring §5.3's method) —
only worth doing if Phase 3 turns up a pattern suggesting this family has the
same problem; not committing to it here without evidence.

### 5.5 Explicitly deferred — the full 789-file spec audit
Given the ~40% stale rate found on a 15-file sample, a defensible estimate is
several hundred specs need a corrected Status line. Doing this by hand,
spec-by-spec, doesn't scale. Recommend (not designed in full here, flagged
for its own follow-up): a batch process — one agent per spec (or per small
batch) checks "does the described feature/file exist in current code?" and
proposes a corrected Status line for human review, rather than auto-editing
686+103 files unattended. This is genuinely a job suited to a fan-out
workflow (many independent, identical-shaped checks) rather than sequential
manual work — flagged as a candidate if the user wants to commit to that
scale of effort, not started here.

## Open questions

1. **Archive-location convention (§2.3/§4.1)** — confirm before touching
   anything: is `docs/specs/archive/` the intended real destination (this
   spec's recommendation), or was the README's `specs/archive/` the actual
   intent and the 26 existing archived files are themselves misplaced and
   need moving? These are different remediations.
2. **Appetite for §5.5's full audit** — this spec deliberately scopes Phase
   1 to the 11-16 files already identified by name, not a commitment to
   fix all ~300 estimated stale specs. Confirm whether that larger effort is
   wanted (and at what scale/budget) before committing to it.

## References

- `docs/specs/README.md` — the stated directory-flow convention this spec
  found inconsistently followed.
- `docs/specs/SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md` — origin of the
  closed Status enum and its own warning about status rot.
- `docs/specs/SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md` — corrected
  earlier this session as a concrete single-file example of exactly the
  pattern this spec quantifies at scale.
- `CLAUDE.md` — contains the stale `specs/`-is-legacy claim this spec
  recommends fixing (§2.4/§4.2).
