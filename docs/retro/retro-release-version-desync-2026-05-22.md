# RETRO — release changelog / version-file desync (v0.38.0)

**Date:** 2026-05-22
**Author:** AgentA
**Severity:** Medium — no data lost; caught before a regressed release shipped, but it blocked the release pipeline and risked publishing a version *below* the changelog's latest entry.
**Area:** release workflow (`task release`, `scripts/bump-wrapper.sh`, `@a5af/bump-cli`), `VERSION_HISTORY.md`, reagent review gate.

---

## Summary

PR **#964** (`chore: release v0.38.0`, squash commit `791add54`) advanced
`VERSION_HISTORY.md` to a `## 0.38.0` section and consumed the pending
changesets — but **never bumped `package.json` / `Cargo.toml`**, which stayed
at `0.37.2`. The repo's two records of "current version" — the changelog and
the version files — silently diverged.

It surfaced ~a day later: the *next* `task release` read `package.json`
(`0.37.2`) and proposed **`0.37.3`** — a release *below* the `0.38.0` already
recorded as shipped. reagent had reviewed #964 and approved it without
catching the mismatch.

## Timeline

- **#964 `chore: release v0.38.0`** merges (commit `791add54`). Its diff:
  ~29 `.changesets/*.md` deletions + the `VERSION_HISTORY.md` `## 0.38.0`
  section. `package.json` / `Cargo.toml` **unchanged — `0.37.2`** before *and*
  after the commit (`git show 791add54:package.json` confirms).
- reagent reviews #964 → **APPROVED**.
- Later: `task release` for the next release reads `package.json` = `0.37.2`,
  proposes `0.37.3`. Caught by hand — a release below the changelog head.

## Impact

- `VERSION_HISTORY.md` says `0.38.0` shipped; the version files say `0.37.2`.
- The next release was blocked: `task release` would regress the version.
- Build artifacts carry an unreliable version label.

## Root cause

The changeset/changelog workflow (RFC #857) split a release into **multiple
artifacts that must stay in lockstep**:

- the consumed `.changesets/*.md`
- the `VERSION_HISTORY.md` section
- `package.json.version` + `Cargo.toml [workspace.package].version`
- the lockfiles (`Cargo.lock`, `package-lock.json`)

`task release` produces all of them — but **nothing enforces that they agree**
once committed. If the version-file bump silently fails — `@a5af/bump-cli` has
a known silent-fail mode (it skips `Cargo.toml` when a description is too long;
see the bump-long-description finding) — `task release` *still* reports
success, *still* appends `VERSION_HISTORY`, *still* consumes the changesets.
The result is a "release" commit whose **changelog says 0.38.0 while the
version files never moved**.

Before the changelog workflow there was effectively **one** record of the
version: the version files, bumped by `bump --commit`. The changelog added a
**second, independent** record — and added no check that the two agree. That
is why this class of failure "started once we moved to changelogs."

## Why it wasn't caught

reagent reviewed a PR literally titled `chore: release v0.38.0` and approved it
without noticing that `VERSION_HISTORY` in that same diff said `0.38.0` while
`package.json` stayed `0.37.2`. The release-consistency invariant was not part
of any review checklist or CI gate — it relied entirely on the operator and a
general-purpose reviewer noticing.

## The invariant

> In every commit, the top version of `VERSION_HISTORY.md` MUST equal
> `package.json.version`, MUST equal `Cargo.toml [workspace.package].version`,
> MUST equal the project versions in `Cargo.lock` / `package-lock.json`.
>
> A PR that advances one of these without the others is a defect.

## Action items

1. **reagent gate (primary — the explicit ask).** reagent must verify the
   release-consistency invariant and return `CHANGES_REQUESTED` on a mismatch —
   especially on any PR that touches `VERSION_HISTORY.md`, `package.json`, or
   `Cargo.toml`. A `chore: release` PR whose `VERSION_HISTORY` head ≠
   `package.json` version is an automatic block. Wire it via `CLAUDE.md` (which
   reagent loads as project context) and/or reagent's own configuration so the
   check is explicit, not incidental.

2. **`task release` self-verify (deterministic).** After `task release` runs,
   it must assert `package.json == Cargo.toml == VERSION_HISTORY-head ==
   lockfiles` and **fail loudly** otherwise. It must not report success on a
   partial bump. Do not trust `bump-cli`'s own "version = X" report — re-read
   the files on disk.

3. **CI guard (deterministic backstop).** A CI step on every PR that checks the
   invariant, independent of the LLM reviewer. An invariant this mechanical
   should be enforced by a script that *cannot* miss it — a reviewer is the
   secondary net, not the primary one.

4. **Fix the `bump-cli` silent failure.** A version-bump tool silently skipping
   a file is the underlying trigger. Make `@a5af/bump-cli` fail loudly when it
   cannot update a target file, or replace the bump step.

## Remediation of the current desync

`package.json` / `Cargo.toml` / lockfiles will be corrected to `0.38.0`
(matching `VERSION_HISTORY`'s latest released entry) as the base of the next
release PR, which then bumps forward normally.
