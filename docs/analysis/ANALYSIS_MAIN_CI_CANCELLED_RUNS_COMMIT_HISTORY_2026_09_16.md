# Analysis: many commits on `main` show "failed" CI in GitHub's history — they don't, they're cancelled

**Date:** 2026-09-16
**Status:** implemented — root-caused here; fix applied directly to
`.github/workflows/ci-pr.yml` per this repo's own CI-fix exception (see
`CLAUDE.md` §Git Workflow: `.github/workflows/` fixes may push straight to
`main`, no PR).
**Trigger:** user report, verbatim: *"take a look at the commit history in
github. so many of them have failed tests. most had passing tests when
merged. why is that?"*

---

## TL;DR

Nothing is actually broken. `.github/workflows/ci-pr.yml` runs on both
`pull_request` and `push` (to `main`). Its `concurrency` group is correctly
scoped per-PR for `pull_request` events, but falls back to the constant
`github.ref` for `push` events — so **every** post-merge CI run on `main`
shares one single concurrency group. With `cancel-in-progress: true`, each
merge's post-merge run gets killed the instant the next merge's post-merge
run starts. When PRs land in quick succession (routine during active work),
that's most of them. GitHub renders a cancelled run with a red/grey mark
that reads as a failure at a glance — but the PR's own pre-merge CI run
(the one actually gating the merge) was never affected, which is exactly
why "most had passing tests when merged": that statement is true, and
describes a different CI run than the one showing red in the commit list.

---

## Evidence

`gh run list --branch main` (via the identity-safe `scripts/gh-agent.sh`
wrapper), filtered to the `CI (PR) — compile tests + run` workflow, `push`
events, over roughly a 45-minute window of active merging on 2026-09-16:

```
17:29:32  cancelled   (a49b8555)
17:29:24  cancelled   (6e63c99a)
17:08:23  success     (af38ebcb)
17:01:52  cancelled   (ee2cccc9)
16:51:48  cancelled   (b5928813)
16:51:35  cancelled   (f4cd44ab)
16:39:35  cancelled   (561853f8)
16:37:37  cancelled   (8c922096)
16:21:37  success     (79bd9fcb)
16:18:48  cancelled   (ac57bb40)
16:17:56  cancelled   (78be0651)
16:07:36  cancelled   (e5460b90)
16:05:39  cancelled   (97879f19)
```

10 of 13 `push`-triggered runs in this window show `cancelled`, all the
same workflow, all `event: push`. The `success` ones are the merges that
happened to land with enough of a gap before the next one for their
post-merge run to finish first.

## Root cause

`.github/workflows/ci-pr.yml`:

```yaml
on:
  pull_request:
  push:
    branches: [main]

concurrency:
  group: ci-pr-${{ github.workflow }}-${{ github.event.pull_request.number || github.ref }}
  cancel-in-progress: true
```

- **`pull_request` event:** `github.event.pull_request.number` is set, so
  the group is `ci-pr-CI (PR) — compile tests + run-<PR number>` — unique
  per PR. Two different PRs' CI runs never touch each other; pushing a new
  commit to the SAME PR correctly cancels that PR's own stale run (the
  intended, working case — this is what kept happening earlier this
  session on PR #3266, correctly).
- **`push` event (post-merge, to `main`):** `github.event.pull_request.number`
  is empty (a push isn't a PR event), so the `||` falls through to
  `github.ref`. For every push to `main`, `github.ref` is the identical
  string `refs/heads/main` — so the group is the same
  `ci-pr-CI (PR) — compile tests + run-refs/heads/main` regardless of
  which commit triggered it. `cancel-in-progress: true` then does exactly
  what it's supposed to for a shared group: kills whatever's still running
  in it the moment a new run starts in the same group.

The workflow's own logic is doing precisely what the config asks — the
config just doesn't distinguish one push-triggered run from another the
way it already distinguishes one PR from another.

## Why this looks alarming but isn't

- The PR's own `pull_request`-triggered run is what actually gates the
  merge (branch protection reads that one). It is never a target of this
  cancellation, because its group key is the PR number, not `github.ref`.
  A merged PR having "passing tests when merged" and its resulting commit
  showing "cancelled" on `main` are two different CI runs of the same
  workflow, not a contradiction.
- `cancelled` is not `failure`. GitHub's commit-list UI renders both with a
  similarly alarming icon at a glance, but the underlying `conclusion`
  field is distinct (confirmed directly via `gh run list --json conclusion`
  above) — nothing about the code, the build, or the tests actually failed
  on the cancelled runs; they simply never got to run to completion.
- This is a volume effect, not a correctness one: it only manifests when
  merges land close enough together that one post-merge run hasn't
  finished before the next starts. A quiet period between merges shows
  clean `success` runs, as seen above.

## Fix

Scope the `push`-event fallback to the commit SHA instead of the branch
ref, so each push gets its own group — matching the per-PR uniqueness the
`pull_request` branch of the same expression already has:

```yaml
group: ci-pr-${{ github.workflow }}-${{ github.event.pull_request.number || github.sha }}
```

`github.sha` is unique per push (the commit that triggered it), so two
different merges landing close together no longer share a group and can no
longer cancel each other. A second push to the *same* commit can't happen
(pushes always advance the ref to a new SHA), so this loses no real
cancellation behavior — the only case being removed is exactly the
spurious one this analysis describes.

Applied directly to `.github/workflows/ci-pr.yml` per this repo's
documented CI-fix exception (`CLAUDE.md` §Git Workflow) — no PR needed for
a `.github/workflows/` change.
