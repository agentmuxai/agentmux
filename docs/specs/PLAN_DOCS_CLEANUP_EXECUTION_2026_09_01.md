# Docs cleanup — execution plan

**Status:** active — A, B, D and E have shipped; C is held pending an owner
decision (§6). See §7 for what actually happened.
**Date:** 2026-09-01
**Owner:** AgentY
**Scope:** `docs/`, top-level `specs/`
**Related:** [`SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md`](./SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md)
(the diagnosis and phase plan this executes — read it first),
[`docs/specs/README.md`](./README.md) (the closed `Status:` vocabulary, Phase 1),
`docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`,
`docs/reports/REPORT_DEAD_CODE_AND_DOCS_SWEEP_2026_08_03.md`,
`docs/reports/REPORT_REPO_HEALTH_AUDIT_2026_07_05.md`

---

## 1. The one thing this plan must not be

**A fourth audit.**

The diagnosis is done and it is good. `SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md`
found the root cause precisely — `Status:` was free text (257 distinct values),
so nothing could tell "current" from "shipped six weeks ago and never updated"
without reading prose. It also recorded the finding that matters more:

> **This exact problem has already been audited twice, with concrete fix
> recommendations, and neither pass's recommendations were applied.** […] Any
> fix here has to not be a third audit that also doesn't stick.

That hardening plan then shipped Phases 0 and 1 — and **Phases 2, 3 and 5 were
never started.** So the pattern repeated a third time: good analysis, partial
execution, drift resumes.

This plan therefore proposes **no new analysis**. Every item below is either
(a) executing an already-agreed unshipped phase, or (b) fixing something that
is broken against a rule the repo has already adopted. If an item requires a
fresh judgement call about a doc's content, it is out of scope by construction.

## 2. Current state (measured 2026-09-01)

| Metric | Value |
|---|---|
| Markdown files under `docs/` | 1,206 |
| `docs/specs/` | 729 |
| Compliant `Status:` first word | **411 (56%)** |
| Non-compliant first word | **192 (26%)** — `spec`, `ready`, `design`, `phase`, `approved`, … |
| No `Status:` line at all | **126 (17%)** |
| `superseded` **without** the required `Superseded-by:` pointer | 2 |
| Duplicate top-level `specs/` tree | 102 files |
| `docs/specs/INDEX.md` last updated | 2026-08-22 |
| `draft` specs untouched >60 days | 141 |

Two things the numbers say plainly:

- **Phase 1 worked where it was applied and stopped.** 411 compliant is real
  progress from "257 distinct values", but 318 files are still outside the
  vocabulary, and nothing prevents number 319.
- **`docs/retros/` is gone.** The duplicate directory both earlier audits
  flagged has actually been merged. Precedent that these items do get done when
  someone executes them.

## 3. What gets done, in order

Ordered by *"would leaving this be actively misleading?"*, not by size.

### Batch A — the docs I made stale today (blocking, do first)

Five docs created in this session are non-compliant, and one is **actively
wrong**:

| File | Problem |
|---|---|
| `SPEC_BROWSER_PANE_CAMERA_ACCESS_2026_09_01.md` | Says **"Not implemented"**. It shipped today across PRs #2893/#2895/#2896/#2897/#2899. |
| `SPEC_TAB_WINDOW_RENDER_ARCHITECTURE_2026_08_31.md` | `Proposal.` — not in the enum. §3.1/§3.2 are rejected, §3.3/§3.4 open. |
| `SPEC_HEADLESS_TRANSIENT_RETRY_2026_08_31.md` | `Proposal.` — not in the enum. Phase 0(a) shipped in #2886. |
| `REPORT_TAB_FLASH_SYSTEMIC_ANALYSIS_2026_08_31.md` | `Analysis,` — not in the enum. |
| `REPORT_TRANSIENT_API_FAILURE_RETRY_STATE_2026_08_31.md` | `Assessment.` — not in the enum. |

I was asked to clean up drift and had spent the same day adding to it. A camera
spec that says "not implemented" the day the feature ships is precisely the
failure the hardening spec opens with — a doc misleading within 48 hours.

Doing this first is also the honest sequencing: fix your own contribution
before proposing anyone else's be touched.

### Batch B — broken against an adopted rule (small, unambiguous)

`docs/specs/README.md` states `superseded` **REQUIRES** a `Superseded-by:` line,
"a broken pointer is worse than none". Two files declare `superseded` with no
pointer:

- `SPEC_COMPOSER_STRIP_LEFT_RIGHT_BALANCE_2026_08_24.md`
- `SPEC_LAYOUT_MINIMIZE_LOCKED_STATE_REDESIGN_2026_07_16.md`

Resolve each by finding the real successor from `git log`.

If no successor document exists, the status is wrong — but **`historical` is not
the automatic answer.** An earlier revision of this plan said it was, and
executing Batch B immediately showed why that is a trap: the minimize spec has
no successor file because §8 of the document *is* the final design, and it
shipped (#2197, extended #2211) with backend layout code still citing it.
Defaulting to `historical` would have replaced one inaccurate status with
another and made a code-anchored, implemented spec look like a mere record.

So: find what the document actually represents *now* — `implemented` if its
design shipped, `historical` if it only records a past effort — and never
invent a `Superseded-by:` pointer to satisfy the enum.

### Batch C — Phase 2, directory consolidation (agreed, never started)

A top-level `specs/` tree holds 102 files alongside `docs/specs/`'s 729. Two
audits recommended consolidating; `docs/retros/` was merged and this was not.

Mechanical: move, fix inbound references, leave no dangling links. **The
content is not reviewed or rewritten** — only relocated. Anything genuinely
superseded moves to `docs/specs/archive/` rather than being deleted, matching
existing convention (28 files there today).

### Batch D — Phase 3, regenerate the index

`INDEX.md` is the tool meant to help agents find the current doc, and the
hardening spec already caught it being "itself an instance of the problem it's
meant to solve". It is 10 days stale and predates every doc in this session.

Generate it from the tree rather than hand-maintaining it — a hand-written
index is guaranteed to rot again, and this is the third time it has.

### Batch E — Phase 5, the part that stops audit #5

Everything above is a one-shot. Without this, drift resumes the day it lands —
which is the documented history of this exact problem, three times over.

Minimum viable enforcement, as a check that runs in CI:

1. Every **added or modified** `docs/**/*.md` **and `specs/**/*.md`** in a PR
   must have a `Status:` line whose first word is in the closed enum.

   Both trees, deliberately. Batch C (which would merge `specs/` into
   `docs/specs/`) is deferrable and gated on an owner decision, so scoping the
   check to `docs/` alone would leave that tree's 135 files unprotected
   *indefinitely* — a new root spec could omit the enum entirely with CI green,
   which contradicts this batch's whole claim of stopping the backlog growing.
   The `specs/` glob is removed only once the move has actually landed.
2. A `superseded` status must carry a `Superseded-by:` pointing at a path that
   exists.

**Deliberately scoped to changed files only.** A repo-wide gate would fail
every PR on 318 pre-existing violations and be disabled within a day. Applied
to the diff, it is always green for compliant work and stops the backlog
growing — which is the actual goal.

## 4. Explicit non-goals

- **No retroactive restamp of the 318 non-compliant/status-less specs.** The
  hardening spec names this as the failure pattern to avoid, and it is: an
  unverified bulk restamp replaces "unknown status" with "confidently wrong
  status", which is worse. Correct statuses require checking each doc against
  code — that is the 08-07 report's batch approach, and it is a standing
  activity, not this cleanup.
- **No deleting the 141 stale drafts.** Age is not evidence of worthlessness;
  several this session turned out to be the only record of a real decision. If
  a doc genuinely has no value it can be archived, but that is a per-doc
  judgement and needs its own pass.
- **No rewriting doc content.** Statuses, locations and pointers only.
- **No new audit report.** If this plan produces a findings document instead of
  a diff, it has failed on its own terms.

## 5. Sequencing and verification

Batches are independent and land as separate PRs, smallest-blast-radius first:
**A → B → E → D → C.**

E before D and C deliberately: the enforcement check is what makes the later,
larger mechanical changes safe to land without reintroducing drift, and it is
cheap. C is last because it touches the most files and is the easiest to defer
without loss if priorities change.

Verification per batch:

- **A, B** — re-run the compliance measurement; compliant count rises by
  exactly the number of files touched, non-compliant falls by the same.
- **C** — no dangling links: every reference to a moved path resolves.
  `grep` for the old paths returns nothing outside archive/history.
- **D** — every `docs/specs/*.md` appears in the index; no index entry points
  at a missing file.
- **E** — the check fails on a deliberately malformed doc and passes on `main`
  as it stands after A and B.

## 6. Open question for the repo owner

**Is the top-level `specs/` tree (Batch C) safe to move?** Two audits
recommended it and nobody has, across six weeks — which might mean nobody got
to it, or might mean something outside this repo references those paths. Worth
one check before moving 102 files; the other batches do not depend on the
answer.

## 7. Execution record

Added after the fact, because a plan that still reads "nothing below has been
executed yet" once four of its five batches have shipped is precisely the drift
this document exists to stop — and it is worse here than elsewhere, since a
reader checking whether the cleanup happened would conclude it did not.

| Batch | PR | Outcome |
|---|---|---|
| Plan | #2906 | This document. |
| A — my own stale docs | #2907 | 5 docs corrected, incl. the camera spec that said "Not implemented" the day it shipped. |
| B — missing `Superseded-by:` | #2909 | 3 docs (one more than the 2 measured), each resolved differently — none defaulted to `historical`, per §3's warning. |
| E — enforcement | #2912 | `scripts/check-doc-status.sh`, plus three `check:*` gates that existed in the Taskfile but had **never run in CI**. |
| D — generated index | #2914 | `scripts/gen-docs-index.sh` + `--check`. |
| C — directory consolidation | — | **Held.** §6 is still unanswered. |

### What the measurements got wrong

§2's table was taken before execution and two entries did not survive it:

- **`superseded` without a pointer: 2 → 3.** The measuring glob was
  `docs/specs/*.md specs/*.md`, which silently skipped `archive/` subdirectories
  and `docs/analysis/` entirely. A verification that cannot see part of the tree
  reports a clean result for the part it can see, which is how "clean" and
  "unchecked" get confused. Later passes used `find docs specs -name "*.md"`.
- **`INDEX.md` "10 days stale"** understated it. Nothing in the index was
  *broken* — all 77 curated entries still resolved — but 165 specs added in the
  previous 30 days were absent. The index was not rotting, it was being outrun,
  which is a different problem and needs a generator rather than an update.

### The failure worth recording

Batch D's first implementation **silently dropped 189 of 728 specs** (26%): it
bucketed each file under the literal first word of its `Status:` line but only
printed the seven canonical buckets, so anything starting `shipped`, `proposal`,
`ready`, `rootcaused`, … vanished with no trace — under a generated header
claiming to cover every file. CI passed on it, and `--check` could not have
caught it at any point, because it diffs against a re-run of the same logic: a
*stable* bug stays green forever.

Caught in review (#2914). The fix that matters is not the missing section but
the completeness assertion added alongside it — emitted rows must equal
candidate files or the script refuses to write. §1 says this plan must not be a
fourth audit; the same logic applies to its tooling. A generator that can
quietly under-report is a new instance of the problem, not a fix for it, and
only an invariant it cannot pass while broken makes that structurally
impossible.
