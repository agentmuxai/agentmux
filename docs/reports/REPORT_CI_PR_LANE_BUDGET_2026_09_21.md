# Report: slimming the Windows PR check, and what "move it to nightly" actually costs

**Date:** 2026-09-21
**Status:** analysis — measured against 32 consecutive `ci-pr.yml` runs
(2026-09-21). No workflow changes made; this is the evidence and the
recommendation, not an implementation.
**Trigger:** Repo owner, after watching the Windows check gate three PRs:
*"we want to slim the PR check to absolute essentials. less important stuff can
go into the nightly."* — then, specifically: *"i mean slimming windows"*.

Two things are discussed throughout, and nothing else:

- **the PR checks** — `ci-pr.yml`, run on every pull request
- **the nightly build** — `ci-nightly-build.yml`, scheduled 07:00 UTC

---

## 1. Measured baseline

Per-job wall clock, 7 consecutive runs across four branches:

| Job | Duration | Gates the merge? |
|---|---|---|
| `check --tests + test (windows-latest)` | **14–15 min** (7/7 runs) | **yes** |
| `check --tests + test (ubuntu-latest)` | 6 min (7/7 runs) | **no** — `continue-on-error: true` |
| `vitest` | 1–3 min | yes |
| `doc status + grep gates` | <1 min | yes |
| `check` (no-coauthor-trailers) | ~5 s | yes |

**The PR wall clock is the Windows job and nothing else.** Everything else
finishes inside 3 minutes. Windows is ~2.5× Ubuntu for *identical* steps.

Not new: the job's own comment documents a 20 → 35 minute timeout raise on
2026-09-18 after warm runs drifted to 18–19 min and *"two PRs were cancelled at
exactly 20m0s and became unmergeable."* That bought headroom, not speed.

### 1.1 The nightly build, measured

The first revision of this report did **not** measure the nightly — every
number above was PR-only. Corrected; 8 consecutive scheduled runs:

| Date | Wall clock |
|---|---|
| 09-21 | 37m52s |
| 09-20 | **43m08s** |
| 09-19 | **43m55s** |
| 09-18 | 40m08s |
| 09-17 | 30m12s |
| 09-16 | 35m55s |
| 09-15 | 37m22s |
| 09-14 | 29m20s |

Range 29–44 min, median ~37. Per-job on the latest run (all four run in
parallel, so wall clock is the max):

| Job | Duration |
|---|---|
| `cargo build + test (windows-latest)` | **37 min** |
| `cargo build + test (macos-latest)` | 29 min |
| `cargo build + test (ubuntu-latest)` | 19 min |
| `vitest` | 2 min |

**The nightly has the same shape as the PR checks: Windows defines the
critical path.** Which is also the good news for making it a true superset —
the sixteen missing steps (§3) total under ~3 minutes and run in parallel, so
adding them costs essentially nothing in wall clock.

## 2. What the Windows check has actually caught: nothing, in 32 runs

Outcome pairing for the two `check --tests` jobs across the last 32 completed
runs:

| Outcome | Runs |
|---|---|
| Windows success + Ubuntu success | 24 |
| Windows cancelled + Ubuntu cancelled | 5 |
| **Windows cancelled + Ubuntu success** | **3** |
| **Windows FAILED + Ubuntu success** | **0** |

Zero Windows-only failures. Its only distinct contribution in this window was
three cancellations — Windows still running when a cancel arrived, or hitting
the ceiling, while Ubuntu had already finished and passed.

**Two honest caveats**, because this is the number the recommendation rests on:

1. `cancel-in-progress: true` means a new push cancels the prior run, so most
   cancellations are benign, not timeouts.
2. 32 runs is a bounded sample. "Caught nothing recently" is not "cannot catch
   anything" — see §4 for why this repo genuinely does have Windows-only risk.

## 3. The nightly is NOT a superset of the PR checks — this breaks the plan

The premise behind "move the less important stuff to the nightly" is that the
nightly is the bigger set. Today it is not.

**Nightly build — 8 steps total:** Defender exclusion, Ninja, Linux build deps,
macOS build deps, `cargo build --release --workspace`, `cargo test --workspace`,
`cargo test -p agentmux-cef`, `vitest run`.

**PR checks — 22 steps.** Present on PR and **absent from the nightly
entirely**:

- RPC bindings are current
- RPC codegen hygiene
- 8 doc gates (status vocabulary, lifecycle ratchet, specs index, spec
  citations, CLAUDE.md references, doc links, directory map, unbuilt-spec)
- 6 grep gates (menu positioning, scrollbar cursor, ligature font, cef-verify
  self-test, input-handler layout-read, muxbus credential store)
- `tsc --noEmit`
- Theme-aware color lint

The nightly supersets the PR only for **Rust compile/test** (and there it is
genuinely bigger: `--release --workspace`, plus CEF) **and vitest**.

So for 16 of 22 PR steps, "move it to nightly" is not a move — there is no such
step there. Removing it from the PR without adding it to the nightly deletes
the check outright. That is worth fixing on its own terms: the nightly
*should* be the superset, and currently a whole class of gates runs nowhere
after merge.

## 4. Why Windows still has real risk, despite §2

This repo has genuine Windows-conditional behavior, so the §2 result should not
be read as "Windows never matters":

- `cfg!(windows)` branches that compile everywhere and *behave* differently. A
  concrete example added this same day: `normalize_working_dir` folds case on
  Windows only, because NTFS is case-insensitive while Linux is not. That
  compiles identically on both and is only meaningfully exercised on Windows.
- Path handling generally (`\` vs `/`, `\\?\` prefixes — see
  `canonicalizePath`'s own comment about a live Windows-only reload bug).
- Windows is the primary shipped target (MSIX, WinGet, portable).

The distinction that matters for slimming: **Windows-only *compile* breakage is
cheap to catch; Windows-only *runtime* breakage is what costs the 15 minutes.**

## 5. Recommendation — slim Windows to a compile gate

**On the PR checks:**

1. **Windows runs `cargo check --tests` only** — drop `cargo test` from the
   Windows job. This keeps every Windows-specific *compile* failure gated
   pre-merge (cfg'd code, platform APIs, path types) at a fraction of the cost,
   since compiling is the cheaper half and test execution is what serializes.
2. **Promote Ubuntu to blocking** (`continue-on-error: false`) and keep its
   full `cargo check --tests` + `cargo test`. It is 6 minutes, it currently
   gates nothing despite costing that, and it becomes the real test gate.
3. **Add the Defender exclusion to the Windows job**, lifted from the nightly
   (`ci-nightly-build.yml` already has a hardened, try/catch-wrapped version).
   The PR checks have no equivalent, and Defender scanning every intermediate
   `.o`/`.rlib` is the leading explanation for the 2.5× gap. Cheap, no coverage
   change, and it makes the remaining compile-only step faster still.

**On the nightly:** it already runs `cargo test --workspace` on
`windows-latest`, so Windows *test* coverage is not lost by step 1 — only its
pre-merge latency. A Windows-only runtime regression would land on `main` and
surface at 07:00 UTC instead of before merge. Given §2 (zero such failures in
32 runs) that trade looks clearly worth it; given §4 it is a real trade, not a
free one.

**Expected result:** PR wall clock drops from ~15 min to roughly the Ubuntu job
(~6 min), with the Windows compile gate finishing inside that window rather
than defining it.

## 6. Fix the superset violation separately (§3)

Independent of Windows, the nightly should gain the steps it is missing — at
minimum RPC bindings/codegen and `tsc --noEmit`, which are cheap and catch real
drift. The doc and grep gates are a judgment call: several are *ratchets*
scoped to changed files, and a ratchet has no meaning in a nightly run with no
PR to attribute new debt to. Those may legitimately belong on the PR only — but
that should be a decision, not the accident it currently is.

## 7. What should NOT move off the PR checks

- **`vitest` (1–3 min)** — cheap, and caught real breakage this week.
- **The doc gates (<1 min)** — sub-minute ratchets; they only fail a PR that
  *adds* debt, which is exactly the mechanism a nightly cannot reproduce.
- **`no-coauthor-trailers` (~5 s)** — it inspects the PR body, which a nightly
  run cannot see at all.
- **Ubuntu `cargo test`** — after step 2 this is the primary regression gate.
  Moving it would mean regressions land on `main` and surface hours later, to
  whoever pulls next rather than whoever wrote them.

## 8. Confidence

- **Measured, high confidence:** §1 timings (7/7 runs); §2 outcome pairing (32
  runs); §3 step inventory (read directly from both workflow files); the
  Ubuntu job being non-blocking; the Defender step existing in the nightly and
  not in the PR checks.
- **Stated by the workflow itself, not independently re-measured:** the
  2026-09-18 timeout history and the `check-rpc-bindings.sh` redundant-relink
  cost, both quoted from the job's comment block.
- **Hypothesis, not proven:** that Defender accounts for the bulk of the 2.5×
  gap. It is the leading candidate and the cheapest to test — measure the
  Windows job before and after rather than assuming it worked.
- **Judgment, not measurement:** §5's trade of pre-merge Windows runtime
  coverage for ~9 minutes per PR. §2 supports it; §4 is the reason it is a
  trade rather than a free win.
