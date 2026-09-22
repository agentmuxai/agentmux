# SPEC: CI check placement protocol — what runs on a PR, what runs nightly

**Date:** 2026-09-21
**Status:** implemented — #3489. The rule below, plus the migration it
implies, both land in that PR: #3490 (slimmed PR lane + nightly superset) and
#3491 (documentation-only PRs skip the build) were squash-merged into its
branch, so the spec and its implementation reach `main` together. Opened as
`proposed`; corrected 2026-09-22 once that stopped being true.
**Trigger:** Repo owner, after the Windows check gated three PRs at 14–15
minutes each: *"this needs to be organized correct with a protocol... the
nightly needs all the parts. the PR version needs to be slimmed down."*
**Measurements:** `docs/reports/REPORT_CI_PR_LANE_BUDGET_2026_09_21.md` — all
timings and the 32-run outcome analysis cited here come from that report
rather than being restated in full.

---

## 0. The rule, in one line

**The nightly runs everything. The PR runs the fast subset that is worth
blocking a merge on.**

Two workflows, and only two, are in scope:

- **PR checks** — `ci-pr.yml`, on every pull request
- **Nightly build** — `ci-nightly-build.yml`, scheduled 07:00 UTC

## 1. Why a protocol rather than a one-off trim

The current split was not decided; it accumulated. Two symptoms prove it:

1. **The nightly is not a superset.** It has 8 steps; the PR checks have 22.
   Sixteen PR steps — RPC bindings, RPC codegen hygiene, 8 doc gates, 6 grep
   gates, `tsc --noEmit`, the theme color lint — exist in **no** scheduled run
   at all. Whatever they protect is unprotected after merge.
2. **The expensive check is the one earning least.** Windows is the sole
   blocking Rust job at 14–15 min while Ubuntu (6 min) is
   `continue-on-error` and gates nothing. Across 32 runs Windows produced
   **zero** failures Ubuntu did not also produce.

Without a stated rule, the next person adding a check picks a file by
intuition, and the imbalance grows. Hence §2.

## 2. Placement rules

Apply in order. The first matching rule wins.

### R1 — Needs PR context → **PR only** (and that is not a superset violation)

Some checks cannot meaningfully run on a schedule, because their input is the
pull request itself:

- `no-coauthor-trailers` inspects the **PR body**. A nightly run has no PR.
- The doc **ratchets** (`check-docs-lifecycle.mjs`, and every gate scoped to
  *changed files*) only fail a change that **adds** debt. On a nightly run
  there is no diff to attribute new debt to, so the ratchet degrades into a
  whole-repo assertion that fails forever on pre-existing debt — which is
  precisely the failure mode `.github/workflows/ci-pr.yml`'s own comment says
  killed three earlier attempts.

These are **legitimate, permanent PR-only checks**. They must be listed
explicitly in the workflow header (§5) so the asymmetry is a decision rather
than an accident.

### R2 — Fast *and* catches common breakage → **PR and nightly**

Budget: **under ~5 minutes**. Currently qualifying: `vitest`, `tsc --noEmit`,
the non-ratchet grep gates, RPC bindings/codegen, and the Ubuntu
`cargo check --tests` + `cargo test`.

### R3 — Slow, or rarely catches what R2 misses → **nightly only**

Everything whose cost is dominated by compiling or platform provisioning:
release builds, CEF, macOS, and **platform-specific *runtime* verification**.

### R4 — Platform-specific compile safety → **PR as a compile gate only**

A platform can break at compile time (cfg'd code, platform APIs, path types)
far more cheaply than it can be fully tested. Where a platform matters enough
to gate merges, the PR runs `cargo check --tests` for it and nothing more; its
test execution lives in the nightly under R3.

## 3. The superset invariant

> Every PR check must also exist in the nightly, except those listed under R1.

This is the part currently violated, and it is the one a reviewer should
enforce on any future CI change. Checked mechanically: the set difference
between the two workflows' steps should equal the documented R1 list.

**Cost of fixing it: effectively zero.** The nightly's wall clock is its
Windows job (37 min of a 37m52s run; macOS 29 min and Ubuntu 19 min run in
parallel under it). The sixteen missing steps total under ~3 minutes and can
hang off the existing Ubuntu leg or a sibling job without moving the critical
path at all.

### 3.1 A check only counts if it can fail the run

"Exists in the nightly" is necessary and **not sufficient**. A step whose
failure cannot fail the workflow satisfies the set-difference check above
while enforcing nothing, so the invariant has to be read as:

> Every PR check must also exist in the nightly **on a leg that blocks**.

This is not hypothetical — it was the first thing to go wrong when
implementing this spec (reagentx P1 on #3489). `RPC bindings are current` was
added to the nightly's `build-and-test` job gated on
`if: matrix.os == 'ubuntu-latest'`, while that job carried
`continue-on-error: ${{ matrix.os != 'windows-latest' }}`. Both statements
look right in isolation; together they placed the repo's only post-merge
RPC-drift gate on the one leg whose failures were swallowed. Drift on `main`
would have gone undetected exactly as before — with the spec, the diff and
the step list all claiming otherwise.

**When auditing the invariant, check two things per step, not one:** that it
exists, and that the job it sits in can actually fail. `continue-on-error` and
a matrix `if:` interact silently, and the failure mode is a green run.

## 4. What this protocol implies for the current workflows

Derived by applying §2, not by preference:

| Change | Rule | Effect |
|---|---|---|
| Windows PR job drops `cargo test`, keeps `cargo check --tests` | R4 | PR wall clock ~15 min → ~6 min |
| Ubuntu PR job becomes blocking (`continue-on-error: false`) | R2 | The real test gate; already paid for, currently gates nothing |
| Add Defender exclusion to the Windows PR job | — | Lifted from the nightly, which already has a hardened version; the PR has none, and it is the leading explanation for Windows being 2.5× Ubuntu on identical steps |
| Nightly gains RPC bindings, RPC codegen, `tsc --noEmit`, color lint, non-ratchet grep gates | §3 | Restores the superset; ~0 wall-clock cost |
| Doc ratchets + `no-coauthor-trailers` stay PR-only, documented as such | R1 | Asymmetry becomes intentional |
| Revisit `timeout-minutes: 35` **last** | — | It was raised to absorb variance against a 15-min baseline; lower it only after re-measuring, since lowering it while slow is what made PRs unmergeable on 2026-09-18 |

## 5. Obligations on future changes

1. A new check states which rule (R1–R4) places it, in a comment at the step.
2. A check added to the PR checks is added to the nightly in the same PR,
   unless it is R1 — in which case it is appended to the R1 list in the
   workflow header.
3. The `timeout-minutes` comment block in `ci-pr.yml` is kept as the record of
   *why* the ceiling is what it is. It has already prevented one wrong fix
   (raising vs. lowering the ceiling depends on whether the cause is runner
   variance or a genuinely slower suite — measure before assuming).

## 6. The trade this accepts, stated plainly

Moving Windows test execution to the nightly means a **Windows-only runtime**
regression lands on `main` and surfaces at 07:00 UTC rather than before merge.

That risk is real for this repo, not theoretical: `normalize_working_dir`
(added 2026-09-21) folds case on Windows only, because NTFS is
case-insensitive and Linux is not. It compiles identically on both platforms
and is only meaningfully exercised on one. `canonicalizePath`'s own comment
documents a previous live Windows-only bug of exactly this shape.

The 32-run evidence (zero Windows-only failures) says the trade is worth ~9
minutes on every PR by every agent. It does not say the risk is zero. If
Windows-only runtime regressions start reaching `main`, the correct response
is to restore a **targeted** Windows test subset on PRs — the path-sensitive
crates only — not to restore the full 15-minute job.
