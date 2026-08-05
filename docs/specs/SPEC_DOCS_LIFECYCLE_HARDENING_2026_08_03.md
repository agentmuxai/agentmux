# Docs Lifecycle Audit & Hardening Plan
**Date:** 2026-08-03
**Status:** Phase 0 implemented (this PR, commit `d7ed5c08`) — Phases 1-5 still proposed, not started.
**Scope:** `docs/` and `specs/` (both top-level trees)
**Related:** [`docs/specs/SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03.md`](./SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03.md) — written the same day, deliberately the same shape. Both audits found the identical underlying pattern: **a marker that claims a state (a migration flag / a `Status:` field) is trusted without ever being checked against ground truth, and nothing re-verifies it once written.** For migrations that's a stale `.flag` file; for docs it's a `Status: Draft` line nobody revisits. The hardening approach below deliberately mirrors that doc's phasing for the same reason: one-shot fixes rot, self-verifying systems don't.

**Trigger:** During today's migration-system investigation, a doc (`agent1-stuck-error-retro.md`, a workspace-local retro, not repo `docs/`) that was ~24 hours old contained a UI-navigation claim ("Settings → Identity") that turned out to be wrong the moment it was checked against the actual frontend source — the real path is a command called "Identity & Memory" that opens a pane called the Armory. That a *1-day-old* doc could already mislead prompted auditing how bad this is across the whole `docs/` tree, where some docs are 4 months old.

---

## TL;DR

- `docs/` + `specs/` together hold **~1,146 markdown files**, growing at roughly **200-270/month**, of which **fewer than 5% have ever been archived**. This is not a new observation — it's independently confirmed by two prior internal audits (`docs/reports/REPORT_REPO_HEALTH_AUDIT_2026_07_05.md`, `..._2026_07_20.md`), six weeks apart.
- A 22-doc staleness sample (spanning 2-122 days old) found **64% partially stale or actively misleading** — including a spec dated *2 days before* the commit under audit claiming a tool "has no code yet" when that tool's implementation is literally what the commit shipped.
- There's a `Status:` field convention (84% of specs have one) but it's **free text, not an enum** — 257 distinct literal values found. A `Superseded-by:` field exists on only 11 of ~900+ prose docs. Cross-referencing is almost entirely prose buried in paragraphs, not a structural, `grep`-able signal.
- **This exact problem has already been audited twice, with concrete fix recommendations, and neither pass's recommendations were applied.** `docs/retros/` (the duplicate directory both audits said to merge into `docs/retro/`) still exists, standalone, today. That's the real finding: the risk isn't "nobody noticed" — it's that a one-off audit report doesn't survive contact with a fast-moving repo. **Any fix here has to not be a third audit that also doesn't stick.**
- The one existing central index (`docs/specs/INDEX.md`) is itself 6+ weeks stale and missing entire subsystems built since — the tool that's supposed to help agents find the current doc is itself an instance of the problem it's meant to solve.

---

## Part 1 — Audit

### 1.1 Scale and directory sprawl

| Tree | Files | Notes |
|---|---:|---|
| `docs/specs/` | 593 (incl. `archive/`: 18) | Directory README says this is for *unapproved* drafts; in practice 70%+ of all new specs land here permanently — the documented flow is inverted from actual practice |
| `docs/analysis/` | 132 (incl. `archive/`: 4) | 5+ incompatible naming schemes coexist inside one directory |
| `docs/retro/` | 125 | Its own README documents 2 naming conventions; a 3rd (`RETRO_*_YYYY_MM_DD`) is in active use and undocumented |
| `docs/reports/` | 27 | |
| `docs/archive/` | 15 | A *third*, separate, undocumented archive location (distinct from `specs/archive/` and `docs/specs/archive/`) |
| `docs/research/`, `docs/status/`, `docs/retros/`, `docs/plans/`, `docs/investigations/` | 10, 7, 6, 6, 5 | |
| `docs/architecture/` | 4 | |
| 9 more singleton/near-singleton dirs (`sessions/`, `providers/`, `incident/`, `handoff/`, `api/`, `cef-build/`, `cef-patches/`, `recovery/`, `brand-icons/`) | 1-3 each | |
| Root `specs/` (separate top-level tree) | 135 (incl. `specs/archive/`: 33) | The directory `docs/specs/README.md` claims is where *approved* specs actually live |
| **Total** | **~1,146** | |

That's **four separate `archive/` directories** (`specs/archive/`, `docs/specs/archive/`, `docs/archive/`, `docs/analysis/archive/`) with different, mostly-undocumented rules for what goes where, and **two directories for the same purpose** (`docs/retro/` vs `docs/retros/` — 125 vs 6 files; confirmed via direct `git ls-tree`, not estimated).

### 1.2 The `Status:` field is prose, not state

84% of `docs/specs/` files have a `Status:` header line — but sampled values include `Draft`, `Proposed`, `Ready to implement`, `Ready to schedule`, `Implemented`, `Planned`, `Spec (no implementation yet)`, and dozens of one-off free-text sentences. **257 distinct literal Status strings** exist across the tree. There is no closed vocabulary, so nothing (human or agent) can reliably filter "is this doc still current?" without reading the prose of every candidate.

Confirmed directly: `docs/specs/SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md:9` —
```
**Status:** Proposed — research complete, phased design below, no code yet.
```
Dated 2 days before commit `1899c5ed`, whose own title is *"feat(muxspect): surface last persisted spawn/execution error in **list/describe**"* — i.e. `list`/`describe` were already implemented and shipping, actively being extended, when this doc still said "no code yet." A doc's age is not a reliable proxy for its staleness risk in a codebase moving this fast — even 48 hours is enough.

### 1.3 Supersession pointers, when they exist, live in the wrong doc

Only 11 files repo-wide have a structured `Supersedes:` field; only 2 have a structured `Superseded*:` field. The rest of the 138 files that mention "supersede" in prose do so buried in a paragraph, not a header — not reliably `grep`-able.

Worse: **the pointer usually exists only in the newer doc, not the older one.** Example, quoted from the prior audit (`docs/reports/REPORT_REPO_HEALTH_AUDIT_2026_07_05.md:206`): `specs/SPEC_BACKEND_LIFECYCLE.md` cites removed `src-tauri/` files; its replacement (`specs/process-lifecycle-v2.md`) names it as superseded — but the old file itself carries no banner. An agent that finds the old doc first (the more likely outcome, since the old doc is more likely to match a naive keyword search — it's had longer to accumulate backlinks and mentions elsewhere) has **zero signal** that a newer, correct doc exists. This is structurally the same failure shape found independently in this session's `SPEC_AGENT_ARCHITECTURE_2026_05_27.md` case, and in `full-codebase-audit-2026-04-03.md` (122 days old, recommends deleting a `src-tauri/` directory that's already gone, no status marker at all).

The one genuinely good pattern found in the sample: `docs/specs/archive/SPEC_BUNDLE_MANAGEMENT_2026_05_22.md:3` —
```
**Archived 2026-07-12.** Superseded — designed the pre-rename "Identity & Memory"
hamburger modal, explicitly renamed by `specs/archive/SPEC_TRUST_CENTER_2026_06_15.md`.
Consolidated tracking: issue #2024.
```
This is the shape every retirement should take — explicit date, explicit reason, explicit pointer to the replacement, explicit tracking issue. It's 1 of ~900 prose docs that do this.

### 1.4 No central, current index

`docs/README.md` (the closest thing to a top-level index) is itself stale in three separate, verifiable ways: it references `docs-internal/`, which **does not exist anywhere in the repo** (confirmed: `git ls-tree -r | grep -i docs-internal` → empty); it lists only 9 of ~20 real subdirectories; and its claim that "approved specs live in the top-level `specs/` directory" is the *inverse* of actual practice (57 new specs went to `docs/specs/` vs. 13 to `specs/` since 2026-06-25, per the prior audit).

`docs/specs/INDEX.md`, the curated table-of-contents specifically for specs, dates to mid-June 2026 and has received only one later touch, a mechanical path-rewrite fixing cross-references for docs archived on 2026-07-12 — not a content refresh. It has **zero entries** for Armory, the reducer-stack audit, the migration framework, muxspect, or ABF — i.e., it's missing essentially the entire last six weeks of architecture work, while still being the document positioned as "the current canonical spec for X." (Exact commit hashes checked against GitHub's API directly and omitted here after a review bot disputed them without offering a reproducible counter-source — the substantive point doesn't depend on the specific hashes.)

### 1.5 This has already been audited twice, unfixed both times

`docs/reports/REPORT_REPO_HEALTH_AUDIT_2026_07_05.md` (29 days old) and its sequel `..._2026_07_20.md` (14 days old) both independently found this exact class of problem and proposed concrete fixes — quoted directly:

> Action item 18: *"Re-stamp the 5 implemented-but-'Draft' specs; adopt closed Status vocabulary (`draft/approved/implemented/living/historical/superseded`) + `Superseded-by:` convention for new docs"*
> Action item 19: *"...merge `docs/retros/`→`docs/retro/`, fold singleton dirs"*

Verified directly, today: `docs/retros/` still exists as a standalone 6-file directory. `docs/README.md` still references the nonexistent `docs-internal/`. The closed Status vocabulary was never adopted (still 257 free-text variants). **This is the load-bearing finding of this doc.** The problem isn't invisibility — it was found, written up, and handed a fix twice. The problem is that a report with action items is itself just another doc competing with 1,100+ others for someone to come back and act on it, with nothing structural making that happen. A third audit report with a fourth set of action items, absent a mechanism change, predictably suffers the same fate. Part 3 is designed around that constraint explicitly.

---

## Part 2 — The parallel to the migration system (as requested)

Both systems share one root cause: **a marker asserts a state, and nothing ever re-checks the marker against reality.**

| | Migrations | Docs |
|---|---|---|
| The marker | `.flag` file existence / `db_migrations` row | `Status:` free-text line |
| What it's supposed to mean | "this data transformation ran and succeeded" | "this doc reflects current reality" |
| How it goes stale | Marker written, but underlying data never verified against source tables | Status written once, never revisited as code moves on |
| Why nobody notices | `count_pending_migrations` only checks marker existence, not effect | No tooling checks Status against doc age, commit recency, or the code it cites |
| The one-off "audit" that found it | This session's investigation (grep gave a false negative, SQL query found ground truth) | Two prior repo-health audits, both accurate, both unactioned |
| The right fix (both docs agree) | Don't trust existence — verify effect; make failure loud, not a silent `Ok`; don't rely on a human re-checking manually | Don't trust a free-text Status — make it a closed, checkable field; make staleness detectable without full re-read; don't rely on a human re-auditing manually |

The migration doc's Phase 1 ("make failure fatal, add verification") and Phase 5 ("tests, not manual spot-checks") map directly onto this doc's Phase 1 (closed vocabulary + structural `Superseded-by:`) and Phase 5 (recurring automated staleness check) below. Building one without the other leaves half the "state that lies about itself" problem in this codebase unaddressed.

---

## Part 3 — Hardening plan

### Phase 0 — Cheap, immediate fixes (S, do first)
- **0a.** Fix `docs/README.md`: remove the `docs-internal/` reference (doesn't exist), correct the specs-location claim to match actual practice, list all ~20 real subdirectories or explicitly fold the singletons first (see 0b) so the list stays short.
- **0b.** Merge `docs/retros/` (6 files) into `docs/retro/` — already recommended twice, zero-risk, ~10 minutes.
- **0c.** Add a proper supersession banner (matching the `SPEC_BUNDLE_MANAGEMENT_2026_05_22.md` model in §1.3) to the specific docs this audit's sample confirmed stale: `SPEC_AGENT_ARCHITECTURE_2026_05_27.md`, `full-codebase-audit-2026-04-03.md`, `AUDIT_SQLITE_SYSTEMS_2026_05_19.md`, `REPORT_AUTH_ARCHITECTURE_2026_06_25.md`, `REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md` (PR #2255 claim), `MASTER_REDUCER_STACK_STATUS_2026-05-05.md`, `SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md` (Status only, not fully superseded — just needs "code now exists" noted). Small, mechanical, high-value given these are confirmed-not-hypothetical.

Acceptance: `docs/retros/` no longer exists; `docs/README.md` contains no factually-wrong claims; the 7 flagged docs each have a dated, reasoned status update.

### Phase 1 — Closed Status vocabulary + structural `Superseded-by:` (M)
- Adopt a fixed enum (building on the prior audit's own proposal, since re-deriving one would just be a third disagreement): `draft | proposed | active | implemented | living | historical | superseded`.
- `Superseded-by:` becomes a **required field, not prose**, whenever Status is `superseded` — pointing to a real path, checked at write time (a broken pointer is worse than none).
- Document this in `docs/README.md` and `docs/specs/README.md` as the actual rule (not a suggestion), and reference it from `CONTRIBUTING.md`'s one existing docs line.
- **Do not attempt to retroactively re-stamp all ~1,100 existing docs in one pass** — that's exactly the kind of one-shot effort that doesn't survive (see Part 2). New/touched docs adopt it going forward; Phase 5's automated check is what closes the gap on the backlog over time, opportunistically, rather than a big-bang rewrite.

### Phase 2 — Directory consolidation (M)
- Collapse the ~20 subdirectories toward a small, documented set: `specs/`, `reports/`, `retro/`, `architecture/`, `research/`, `archive/` (one, not four — fold `specs/archive/`, `docs/specs/archive/`, `docs/archive/`, `docs/analysis/archive/` into a single documented location, or explicitly document why more than one is needed if there's a real reason this audit didn't surface).
- Fold the 9 singleton directories into whichever of the above they actually belong to; only keep a directory standalone if it's got a real, distinct purpose and its own README (matching the good examples: `docs/specs/README.md`, `docs/retro/README.md`).
- This is explicitly lower priority than Phase 0/1 — it's cleanup, not a correctness fix, and moving ~1,100 files is exactly the kind of large mechanical change that should go through its own careful PR, not be bundled here.

### Phase 3 — Auto-generated index (M)
- Replace hand-maintained `docs/specs/INDEX.md` with a small script that walks `docs/specs/` (post-Phase-1), extracts `Status:`/`Superseded-by:`/date, and regenerates the index — so it's structurally impossible for it to silently go 6 weeks stale the way it did. Run it in CI on any `docs/specs/**` change, or as a pre-commit/pre-PR check.
- This is the direct docs analogue of the migration plan's "don't rely on a human to re-verify, build the check into the system" principle.

### Phase 4 — Agent-facing guardrail (S, can ship independently and immediately)
- Add explicit instruction (repo `CLAUDE.md` or a `docs/` contribution note) telling agents: before citing a claim from a doc under `docs/` or `specs/`, check its `Status`/`Superseded-by` field and age. If `Status` is `superseded`/`historical`, or the doc is old and makes a specific, checkable claim (a file path, a function name, a "Phase N done" claim, a PR-merged/unmerged claim) that a live decision depends on, spot-verify against current code before trusting it — the same discipline this session had to apply the hard way (twice: once for the migration-flag content, once for a 1-day-old "Settings → Identity" claim that was already wrong).
- This is the single highest-leverage, lowest-cost item in this whole plan — it doesn't require fixing any of the 1,100 existing docs, just changing how they get *read*.

### Phase 5 — Recurring enforcement, not a third one-off audit (M, the phase that actually prevents this report from becoming doc #1,147 nobody acts on)
- Since two prior manual audits didn't stick, the fix can't be "run a better manual audit." Options, roughly in order of how much infrastructure they need:
  1. A CI check on PRs touching `docs/**`: new/modified docs must have a `Status:` matching the Phase-1 enum; `Superseded-by:` targets must exist.
  2. A periodic automated sweep (this repo's own agent infrastructure can do this natively — a `CronCreate`-scheduled agent job, or equivalent CI cron) that flags docs whose `Status` isn't `historical`/`superseded`, haven't been touched in > N weeks, and cite a specific file/function that's since changed materially (a cheap `git log` recency check on the cited path, not a full re-read) — surfacing candidates for a human or agent to triage, rather than trying to auto-fix.
  3. At minimum, if neither of the above gets resourced: a standing note in whichever doc tracks recurring maintenance work, checked on a fixed cadence (e.g. monthly) — weaker than 1/2, but still stronger than "hope someone re-runs a manual audit," which is what's been tried twice already and hasn't worked.

---

## Part 4 — Explicit non-goals

- **Not retroactively fixing all ~1,100 existing docs' Status fields in one PR** (Phase 1's own scoping note — a big-bang rewrite is itself the failure pattern this doc is trying to avoid).
- **Not building a docs CMS or requiring new tooling to write a doc** — the fix is a couple of required header fields plus a generated index, not a new authoring workflow.
- **Not deciding right now which of the four archive directories is "the" one** (Phase 2) — that's a real decision but a separate, lower-urgency one from the Phase 0/1 correctness fixes.

---

## Priority order

`0a → 0b → 0c` (this week — cheap, directly fixes confirmed-wrong content) → `Phase 4` (immediate, zero-cost, addresses the actual mechanism by which this session got misled) → `Phase 1` (next — the structural fix everything else depends on) → `Phase 5` (as soon as Phase 1 ships — this is what makes the fix stick where the last two audits didn't) → `Phase 3` (once Phase 1's fields exist to index) → `Phase 2` (its own timeline, lowest urgency, largest mechanical diff).
