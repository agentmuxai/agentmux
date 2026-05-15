# SPEC: Multi-Agent Version Coordination

**Status:** Draft
**Date:** 2026-05-15
**Author:** AgentA (drafted after #850/#854/#853 stack rounds 1–11)
**Tracking:** TBD (GitHub Discussion to be opened with this content)

---

## 1. Problem statement

AgentMux is developed by 3+ AI agents concurrently. Every PR bumps a patch version
via `@a5af/bump-cli`, which writes the new version into **9 files**:

- `package.json`
- `package-lock.json`
- `Cargo.lock`
- `agentmux-srv/Cargo.toml`
- `agentmux-cef/Cargo.toml`
- `agentmux-launcher/Cargo.toml`
- `agentmux-common/Cargo.toml`
- `agentmux-bashwrap/Cargo.toml`
- `VERSION_HISTORY.md`

When two agents both run `bump patch` against the same base commit, they pick
the same next version number, push, and one push wins. The losing branch must
forward-merge or rebase — every one of those 9 files conflicts.

This session alone produced these incidents on the OAuth PR stack:

| Incident | Cost |
|---|---|
| PR #847 merged into stack-base instead of main (#850 head branch stayed alive, downstream PRs auto-closed) | Required PR #854 rescue + retargeting |
| Round-9 forward-merge of main→#853: `git checkout --ours` only listed 3 of 8 conflicted version files, the other 5 kept `<<<<<<<` markers, `git add -A` committed them, `bump verify` returned "consistent" (false negative) | Reagent P0 — build broken on `cargo metadata` |
| Version downgrade flagged: forward-merge took our 0.33.891 over main's 0.33.892 | Reagent P1 every time |
| Reagent's 358-second review on `249081ba` race-stamped to wrong commit during a concurrent push | One wasted review cycle, confusing CHANGES_REQUESTED feedback |
| Codex quota exhausted on a5af account at 02:29Z, no recovery for 5+ hours | #847 had to be merged on reagent-only signal |
| Stale CHANGES_REQUESTED reviews on long-superseded commits block merge because `dismiss_stale_reviews=false` | Every PR requires explicit re-APPROVAL on latest commit; `LGTM`-as-COMMENTED doesn't count |
| Lockfile regen produces ~130-line diffs that re-conflict on every iteration | Adds noise to every PR's diff |

The PR sequencer loop on #853 went through 11 rounds across 5+ hours — most of
those rounds were not novel bot findings but variations of the same
version-coordination friction.

## 2. Goals

- **G1** Reduce per-PR conflict surface from 9 files to ≤2 (ideally 1).
- **G2** Eliminate the "version downgrade" P1 class entirely.
- **G3** Make `git checkout --ours/--theirs` failure modes loud (or unnecessary).
- **G4** Stop accumulating stale CHANGES_REQUESTED reviews after every push.
- **G5** Don't break reproducible builds — lockfiles must stay accurate.
- **G6** Don't require all agents to coordinate verbally before each bump.

## 3. Non-goals

- Replacing `@a5af/bump-cli` itself (the tooling is fine; the **policy** of
  when each PR bumps is what's broken).
- Removing branch protection or weakening review requirements.
- Switching to release-train / monthly cadence — feature PRs continue to ship
  daily.
- Multi-repo (this is single-repo).

## 4. Current state (mechanical)

### 4.1 Version sources

`.bump.json` lists 9 targets all driven from `package.json`'s `version` field.
On `bump patch`, bump-cli:

1. Increments the version in `package.json`.
2. Writes it to all 9 target paths.
3. Regenerates `Cargo.lock` via `cargo generate-lockfile` (does NOT regen npm
   lockfile — that's a separate manual step, see `feedback_use_bump_wrapper_script.md`).
4. Stages those files and creates a commit `chore: bump version to X.Y.Z`.

### 4.2 Cargo workspace

`Cargo.toml` declares 5 members. Each member has its own `version = "..."` line
in its `Cargo.toml`. Cargo supports **workspace inheritance** since 1.64:

```toml
# Root Cargo.toml
[workspace.package]
version = "0.33.894"

# Member Cargo.toml
[package]
version.workspace = true
```

We don't use this. Adopting it collapses 5 Cargo.toml version conflicts to 1
(the workspace root).

### 4.3 Branch protection on `main`

```
required_approving_review_count: 1
dismiss_stale_reviews: false
require_code_owner_reviews: false
require_last_push_approval: false
enforce_admins: false
```

`dismiss_stale_reviews: false` is what causes stale CHANGES_REQUESTED to stick.

### 4.4 Bot review triggers

- **Reagent** fires on `synchronize` (push to head branch) and `edited` (PR
  description change). Doesn't fire on merge commits in some cases (observed
  on `bd54f603`).
- **Codex** fires only when `@codex review` is commented by the `a5af`
  account. Quota is finite per-account-per-day.

## 5. Proposed changes (phased)

### Phase 0 — Loud-fail conflict markers (1 commit, no design risk)

Add a pre-commit hook (or CI check) that fails if **staged changes** contain
unresolved merge conflict markers.

```bash
#!/bin/sh
# .githooks/pre-commit
if git diff --cached --check; then exit 0; fi
echo "Aborting: unresolved merge conflict markers in staged changes." >&2
exit 1
```

**Use `git diff --check`, NOT a `grep -rn '<<<<<<< HEAD'`.** Git's `--check`:

1. Only inspects **lines being added** in the staging area — not all tracked
   files. So a spec file that legitimately contains the literal string
   `<<<<<<< HEAD` as documentation (this very file, §9 examples) doesn't
   trigger the hook.
2. Also catches whitespace errors (trailing whitespace on new lines, mixed
   tabs/spaces) for free.
3. Is what git's own commit machinery uses internally.

**Precedent:** `scripts/verify-version.sh` (PR #16) tried the brittle approach
of grep-scanning every file for version-like strings. It false-positived on
`.rs` test fixtures (`0.12.15`, `0.19.0`, `v0.10.4`) which are unit-test
values, not real references. The fix (`97bf56bf`) made it `continue-on-error`
in CI, and PR #54 (`317bb414`) deleted it entirely in favor of `bump verify`
(which only checks declared targets). Don't repeat that pattern here — `git
diff --check` is the right tool because it operates on the *diff*, not the
full tree.

Also wire the same check to CI as the first job step so it catches anything
that bypasses local hooks (e.g. `git commit -n`).

**Cost:** ~10 LOC + hook install instructions in `BUILD.md`.
**Risk:** Zero — git already runs this check internally; we're just enforcing it.
**False-positive surface:** None for `git diff --check`; the brittle grep
alternative would false-positive on docs/specs containing the strings as
prose.
**Effect:** Eliminates the round-9 P0 class (committed conflict markers in
unlisted files post `git checkout --ours`).

### Phase 1 — Cargo workspace version inheritance (1 PR)

Move `version` from each member `Cargo.toml` to the workspace root's
`[workspace.package]`. Each member references it with `version.workspace = true`.

Update `.bump.json` to point at the workspace root only.

**Files affected:**
- `Cargo.toml` (add `[workspace.package]`)
- 5× `agentmux-*/Cargo.toml` (replace `version = "..."` with `version.workspace = true`)
- `.bump.json` (remove 5 target entries, keep root + package.json + lockfiles)

**Cost:** ~30 LOC.
**Risk:** Low; standard Cargo pattern.
**Effect:** Conflict surface drops from 9 → 4 files (root Cargo.toml, package.json,
Cargo.lock, package-lock.json + VERSION_HISTORY.md).

### Phase 2 — Don't bump in feature PRs (changeset-style)

Adopt the [Changesets](https://github.com/changesets/changesets) pattern:

- Feature PRs add a **changeset file** (`.changesets/<hash>.md`) instead of
  bumping version. Format:

  ```markdown
  ---
  type: patch
  ---
  fix(auth): cancel in-flight session on selection change
  ```

- A `release` PR (opened by a bot on schedule, or manually) consumes all
  pending changeset files, bumps version, regenerates lockfiles, updates
  `VERSION_HISTORY.md`, and merges to main.

This eliminates the version-bump conflict entirely — feature PRs only touch
their own `.changesets/<hash>.md` file (unique hash per PR = no conflict).

**Cost:**
- Bot config (~1 day setup, can use existing tools or hand-roll).
- Reagent rule update: don't flag missing version bump on feature PRs.
- Tooling migration: agents stop running `bump patch`; they run
  `bump changeset` or just write the changeset file directly.

**Risk:** Medium — depends on whether the release PR can be auto-merged or
needs human review (acceptable either way).

**Effect:** Conflict surface drops to **zero** version-files per feature PR.
Lockfile noise concentrates on one release PR.

### Phase 3 — Enable `dismiss_stale_reviews`

Flip `dismiss_stale_reviews: false` → `true` in branch protection on `main`.

This means: once you push a new commit, prior CHANGES_REQUESTED reviews are
**dismissed**. The bots will re-review the new commit; only the latest review
counts.

**Cost:** 1 API call.
**Risk:** Low — bots re-review on every push anyway; manual reviewers might
need to re-approve, but we have zero human reviewers on these PRs in practice.
**Effect:** Eliminates the "old CHANGES_REQUESTED blocking after fix" class
that consumed multiple rounds on #853.

### Phase 4 — Pre-merge lockfile regen bot

A bot (or GitHub Action) that runs `npm install --package-lock-only` and
`cargo generate-lockfile` as the LAST step before merge, then auto-commits
the regenerated lockfile.

**Cost:** ~1 day GitHub Action workflow.
**Risk:** Low if scoped to lockfile-only commits.
**Effect:** Lockfile in HEAD always matches the source files post-merge, so
forward-merges don't regenerate the lockfile (it'll match).

### Phase 5 — Auto-rebase on green

Once the PR is green (all required reviews + checks), have a bot rebase onto
latest main and re-run checks. If anything fails, it stops; otherwise it
merges.

This catches the version-downgrade case: rebasing takes main's version, then
the changeset bot in Phase 2 computes the right next version.

**Cost:** Maybe use `mergify.io` or hand-roll GitHub Action.
**Risk:** Low.
**Effect:** Eliminates the "PR was green at base X, but main has moved" class.

## 6. Migration plan

The phases are **independent** and can ship out-of-order. Recommended order
(by effort/impact):

1. **Phase 0** (conflict-marker check) — ships today, ~10 LOC.
2. **Phase 3** (dismiss_stale_reviews) — single API call, immediate benefit.
3. **Phase 1** (Cargo workspace version) — ~30 LOC, eliminates 5 of 8 files.
4. **Phase 2** (Changesets) — bigger lift, eliminates the rest.
5. **Phase 4** (lockfile bot) — nice-to-have once Phase 2 lands.
6. **Phase 5** (auto-rebase on green) — capstone.

Phases 0+3 alone would have saved an estimated 4+ rounds on the OAuth stack
this session.

## 7. Open questions

- **Q1**: Does anyone manually edit version numbers today (e.g., for
  pre-release tags)? Phase 1 should preserve that path.
- **Q2**: What's the existing reagent rule that requires version bumps on
  feature PRs? Phase 2 needs that rule disabled.
- **Q3**: Is there appetite for a release-PR bot, or do we want a human
  toggle? (Recommend: human-triggered weekly, automated when we trust it.)
- **Q4**: Codex quota exhaustion is unrelated to versioning but is the other
  big friction. Worth a separate spec on bot-account rotation?

## 8. Out of scope (for now)

- **Build-time version injection** (no version in source at all, read from
  env). Possible long-term but breaks every `--version` flag and log timestamp
  that reads from `CARGO_PKG_VERSION`.
- **Trunk-based development with feature flags** instead of feature branches.
  Different ergonomic; not necessary if conflict surface is small enough.
- **Single GitHub account for all agents.** Multiple accounts is intentional
  for codex rotation and audit clarity; combining them creates worse problems.

## 9. Appendix: incident timeline (this session)

| Time (UTC) | Incident | Phase that would have prevented it |
|---|---|---|
| 06:14:19 | #847 merged to stack-base (not main) — branches stayed alive | Out of scope (PR base re-targeting on parent merge — separate spec) |
| 07:00:05 | #854 merged (rescue PR for #847) | — |
| 07:05:57 | Forward-merge main→#853 committed `<<<<<<<` markers in 5 Cargo files | **Phase 0** |
| 07:05:57 | Same merge: version downgrade 0.33.892→0.33.891 (we took --ours) | **Phase 1+2** |
| 07:22:45 | Reagent re-reviewed 6e5cd79b, still flagged P0 markers + P1 downgrade | **Phase 0+1** |
| (ongoing) | Stale CHANGES_REQUESTED on old commits keep branch-protection blocked | **Phase 3** |

---

🤖 Generated with [Claude Code](https://claude.com/claude-code) during the OAuth PR stack.
