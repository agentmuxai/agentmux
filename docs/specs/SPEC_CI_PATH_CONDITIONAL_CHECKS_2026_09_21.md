# SPEC: skip the build on documentation-only PRs, safely

**Date:** 2026-09-21
**Status:** implemented — #3491 (`scripts/ci-classify-changes.mjs` + its unit
tests, and the `changes` job in `.github/workflows/ci-pr.yml`). **Corrected
2026-09-22 (#3507):** #3491 shipped with a real regression this spec did not
anticipate — see the erratum in §1 and §5's now-outdated "No `ci-gate`
aggregate job" bullet. Original text preserved below; corrections are called
out inline rather than silently edited.
**Trigger:** Repo owner: *"can builds be conditional on files? if the PR only
has md files, it should require no builds."*
**Builds on:** `SPEC_CI_CHECK_PLACEMENT_PROTOCOL_2026_09_21.md` (what belongs
on a PR versus the nightly). This spec is orthogonal: that one decides *which*
checks exist, this one decides *when* they need to run at all.

---

## 1. The mechanism, and the trap next to it

GitHub has two ways to not run a check, and they behave oppositely:

| How | What the check reports | Effect on a required check |
|---|---|---|
| Workflow-level `paths:` / `paths-ignore:` / `[skip ci]` | **nothing — stays Pending** | **PR blocked forever** |
| Job-level `if:` conditional | **skipped** | **counts as passing** |

GitHub's own documentation is explicit on both halves: successful check
statuses are *success, skipped, and neutral*, and — directly — *"you should
not use path or branch filtering to skip workflow runs if the workflow is
required to pass before merging."*

This matters here specifically: `ci-pr.yml`'s header records that
`"check --tests + test (windows-latest)"` and `"vitest"` are intended as
required status checks, and that job names are frozen so the branch-protection
rule keeps matching them.

> **Erratum, 2026-09-22 (#3506, fixed by #3507):** this paragraph missed a
> case, and the miss reached production. `"frozen job names"` does not help a
> MATRIX job: when a matrix job is skipped by a job-level `if:`, GitHub
> reports one check under the job's **unexpanded** name (`"check --tests +
> test (${{ matrix.os }})"`, the literal template string) — never the
> **expanded** per-leg name (`"check --tests + test (windows-latest)"`) that
> branch protection was actually pinned to, which only exists once the job
> runs. A docs-only PR that skips `rust` therefore never produces the
> expanded context, and is permanently `BLOCKED` regardless of review or
> re-runs — live example #3505, the first genuinely docs-only PR after #3491
> shipped. §5's "No `ci-gate` aggregate job" bullet below is corrected to
> match: that aggregate now exists (`ci-required` in `ci-pr.yml`), branch
> protection points at it instead of the matrix leg, and this exact class of
> bug can no longer recur regardless of what jobs get renamed or added later.

**Therefore: job-level `if:` only. The workflow-level `paths:` key is banned in
`ci-pr.yml`,** and this spec exists partly to make that a written rule rather
than folklore.

Consequence worth stating: because skipped counts as passing, **the skip
decision is itself safety-critical**. A wrong answer does not fail loudly — it
produces a green check on code nobody compiled. Everything in §2 follows from
that.

## 2. Safety rules

### R1 — "ALL files are docs", never "ANY file is a doc"

The question is *"is every changed file documentation?"*, not *"did any
documentation change?"* Most off-the-shelf path filters answer the second.
A PR touching `README.md` **and** `agentmux-srv/src/lib.rs` must run
everything; a filter answering "any" would say "docs changed → skip".

### R2 — Default to running

Any uncertainty — API error, empty file list, an unrecognised path, a
pagination failure — resolves to **run the build**. The failure mode of this
system must be a wasted six minutes, never an untested merge.

### R3 — CI configuration is never documentation

A change under `.github/`, `scripts/`, `tools/`, or the Taskfile must run
everything. Otherwise a PR editing the test configuration could skip the very
tests it edits. This is the single most dangerous misclassification available,
because it is self-concealing.

### R4 — Only pull requests are eligible

`ci-pr.yml` also runs on `push: branches: [main]`. Post-merge runs always do
the full job: that is the last line of defence before a release, the volume is
low, and it removes any dependency on diffing against the right base.

### R5 — A failed classifier must not become a silent skip

The rules above assume the `changes` job produces an answer. If it *fails*
— runner death, API outage, a bad edit to the script — its dependents are
skipped by default, and skipped counts as passing. The failure mode of the
safety mechanism would be the exact outcome it exists to prevent.

Two inversions in the `if:` close this, and both are load-bearing:

```yaml
if: ${{ !cancelled() && needs.changes.outputs.rust != 'false' }}
```

- **`!= 'false'`, not `== 'true'`.** When `changes` fails or is skipped its
  outputs are the empty string. Empty is not `'false'`, so the build runs.
  Only an explicit, successfully-computed `false` skips anything.
- **`!cancelled()`, not the default implicit `success()`.** Without a status
  function, GitHub skips a dependent job whenever its `needs:` failed — so
  the `if:` would never be consulted at all. `!cancelled()` lets the job
  evaluate after a failed `changes` while still respecting a real
  cancellation.

Written the obvious way (`== 'true'`, no status function), a broken
classifier would skip the build and report the PR green. The `changes` job
additionally never exits non-zero: every internal failure path writes
"run everything" itself, so this is a second line of defence, not the first.

### R6 — The decision is code, and it is tested

The classification lives in `scripts/ci-classify-changes.mjs` with unit tests,
not in a YAML expression. YAML conditionals cannot be tested, and R1–R3 are
exactly the kind of logic that is wrong in ways nobody notices — the whole
point is that a wrong answer is invisible. A reviewer can read the tests.

## 3. Design

```
┌──────────────┐   file list via gh api (no git history)
│   changes    │──────────────┐
└──────────────┘              │  outputs: rust, frontend, docs_only
         │                    ▼
         │        scripts/ci-classify-changes.mjs
         ▼
  rust job    if: !cancelled() && needs.changes.outputs.rust     != 'false'
  vitest job  if: !cancelled() && needs.changes.outputs.frontend != 'false'
  docs job    ALWAYS RUNS
```

Note the `if:` shape — `!= 'false'` and `!cancelled()`, per R5. `== 'true'`
is the natural way to write this and is unsafe.

**The `changes` job never diffs against a base.** It asks the API for the PR's
file list (`gh api repos/{owner}/{repo}/pulls/{n}/files --paginate`). This
sidesteps the `fetch-depth` class of bug entirely — the default shallow clone
has no parent commit to diff against, which is a common and quiet failure in
this pattern. It does take a checkout, but a `sparse-checkout` of the single
classifier file with no history, which costs nothing.

**The `docs` job always runs.** Its gates (doc status vocabulary, the lifecycle
ratchet, spec citations, link resolution) are precisely what a documentation
PR needs checked. Skipping them on a docs-only PR would invert the intent.

### Classification

`rust` is true when any changed file is **not** provably irrelevant to the Rust
build. Same for `frontend`. Concretely, a path is docs-only when it matches:

- `docs/**` — except that a doc change still runs the `docs` job, which always runs
- `**/*.md`, `**/*.mdx`
- `.changesets/**`
- `LICENSE`, `NOTICE`, `**/*.txt` under `docs/`

Everything else — including any path this list does not recognise — means run
(R2).

## 4. Expected effect

A documentation-only PR drops from ~15 min (the Windows leg, per
`REPORT_CI_PR_LANE_BUDGET_2026_09_21.md`) to the `changes` job plus the `docs`
job: **well under a minute**, with `rust` and `vitest` reporting *skipped*,
which branch protection accepts.

A PR touching any code is completely unaffected.

## 5. What this deliberately does not do

- **No dependency-graph awareness.** A path filter cannot follow imports. This
  design does not try: it only ever declares something docs-only, and
  documentation is not imported by Rust or TypeScript. The shared-code problem
  that breaks naive monorepo filters does not arise, because the only thing
  being skipped is "everything", on the basis of "nothing here is code".

  **This rests on one assumption, verified 2026-09-21: no build input reads a
  markdown file.** Checked directly — no `include_str!("*.md")` in any crate,
  and no `.md` import in the frontend. The residual risk is that someone later
  adds one (`#![doc = include_str!("../README.md")]` is the usual way it
  appears, and it makes a `.md` edit genuinely able to break the build). If
  that happens, a doc-only PR touching that file would skip a build it should
  have run. The cheap guard, if it ever seems warranted, is a grep gate in the
  `docs` job — deliberately not added now, since adding an untriggered gate
  for a hypothetical is its own cost.
- **No `ci-gate` aggregate job.** *(Corrected 2026-09-22 — this decision is
  no longer current; see the erratum in §1. Original reasoning preserved
  below for the record.)* The widely-recommended terminal aggregate
  (`needs:` everything, `if: always()`) would let branch protection point at a
  single context. It is a genuine improvement but requires repointing branch
  protection, which needs repository-admin access this change does not have.
  Noted as a follow-up, not silently skipped. Note also the trap if it is
  added later: an aggregate job that can itself be skipped always reports
  success, so it needs a level of indirection to be meaningful.

  **What actually shipped (#3507):** the "follow-up" arrived sooner than
  expected — as the fix for the exact `if:` correctly warns about above. This
  spec deferred the aggregate as a nice-to-have; the trigger for actually
  building it was `rust` being a matrix job, which made the deferred case a
  live production bug instead of a hypothetical one. `ci-required` in
  `ci-pr.yml` `needs: [changes, rust, frontend]`, runs unconditionally
  (`if: always()`), and avoids exactly the trap this bullet warned about: it
  does NOT rely on GitHub's default `needs`-success gating (which would fail
  it on any legitimately skipped leg) — `scripts/ci-required-check.mjs`
  inspects each dependency's own `result` explicitly, treating `success`/
  `skipped` as passing and everything else (`failure`, `cancelled`, an
  unrecognized value) as failing closed. Branch protection was repointed at
  this one job in the same change.
- **No third-party action.** `dorny/paths-filter` is the usual choice and is
  fine, but it answers "ANY" (R1), needs SHA-pinning and extra permissions, and
  would put the safety-critical logic somewhere untestable. A dozen lines of
  `gh api` plus a tested script is less machinery and more verifiable.
