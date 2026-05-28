# SPEC: Local-build versioning — stop committing bumps for smoke builds (2026-05-28)

**Author:** AgentA
**Status:** Proposal — chosen pattern, ready to implement.
**Reporter:** user (this session) — "we want a reliable way to ensure we can iterate on patches during local dev work .. we tried other stuff, but it keeps breaking, can u research best practices."
**Affected:** `Taskfile.yml` (`package`, `package:local`, `dev:local`), `scripts/package-portable.sh`, `scripts/bump-wrapper.sh` (no longer called by package), `agentmux-common/src/data_paths.rs` (data-dir keying), `CLAUDE.md` (the versioning section).
**Supersedes:** the `task package` auto-commit-bump introduced to replace the old ephemeral `package:local`.
**Related:** RFC #857 (changesets), `SPEC_MULTI_AGENT_VERSION_COORDINATION_2026_05_15.md`, `SPEC_DATA_CHANNELS_2026_05_24.md`, `docs/retro/retro-release-version-desync-2026-05-22.md`.

---

## 1. The recurring failure

`task package` step 1 is `bash scripts/bump-wrapper.sh patch -m "build" --commit` — it **commits a global release-version bump, on the current branch, before any build step can fail.** That one decision produces three distinct, recurring breakages:

1. **Cross-branch version collision.** The "monotonic counter" is only monotonic on a *linear* branch. Two feature branches forked from `main@0.39.2` both bump to `0.39.3` → same version → same Desktop folder name → same data dir. The first build's running instance locks the folder; the second build's `package-portable.sh` refuses. (This is exactly what happened building the send-now portable while the diagnostic v0.39.3 was running.) The normal multi-agent state of this repo *is* parallel branches, so this isn't an edge case — it's the default.

2. **Stranded commit on failure.** Bump is step 1; packaging is the last step. Any failure after step 1 (folder lock, disk, cargo) leaves a `chore: bump version` commit in git for a build that never shipped. The diagnostic branch is carrying such a commit (`46b7c86b`) right now.

3. **Fights the changesets contract.** RFC #857: feature PRs must NOT bump — they add a changeset; the release PR consumes them. But a smoke build commits version-file mutations onto the feature branch, which then pollute the PR (tripping reagent's release-invariant gate) or must be manually reverted every iteration. `feedback_incremental_version_bumps` is literally a standing note to do this revert dance.

**Root cause:** a local smoke build mutates committed *global release* state. The label a developer needs ("which ZIP is which") was conflated with the release version. `task dev:local` already avoids this (ephemeral bump, restored on exit, no git mutation); `task package` is the lone holdout that commits.

## 2. What the industry does (research synthesis)

Three independent best-practice sources, all aligned:

- **Changesets (their own docs + LogRocket/Infinum/Vercel guides):** version bumps happen *only at release time*, computed as the max accumulated bump across changeset files. Feature branches never touch version files. → validates RFC #857; indicts the package auto-bump as the deviation.
- **Semver §10 + dev.to "the unknown buildMetadata":** build metadata (`1.0.0+sha.5114f85`) is **ignored when determining precedence**. Two versions differing only in build metadata are equal. → a local label in the `+...` field can *never* collide with or reorder the release version. Guidance is explicit: put git revision / build timestamp / build ID in metadata, *not* in the semver core.
- **`git describe --tags --dirty --always` (git-scm, GitVersion, versioneer, setuptools-git-versioning):** the standard local-build identifier. Format `TAG+DISTANCE.gHEX[.dirty]`, e.g. `0.39.2+17.g9dd2d78.dirty`. Traceable (embedded sha → check out the exact source), ordered (distance = commits since tag), and `.dirty` flags an uncommitted working tree — *the exact "iterate on a patch without committing" loop the user lives in.*

The consistent principle: **Semantic versioning is about releases, not builds.** Builds get *identifiers*; releases get *versions*. Conflating them is the documented anti-pattern, and it's precisely what broke here.

## 3. Chosen pattern

**`task package` stops bumping and stops committing. It derives an ephemeral build label and stamps it into the artifact; git is never written. The release version moves only through `task release` (changesets).**

### 3.1 The build label

```
<base>+<git-describe>.<build-stamp>
```

- `<base>` — the current `package.json` version, read live, never mutated. e.g. `0.39.2`.
- `<git-describe>` — `git describe --tags --dirty --always` output's suffix: `<distance>.g<sha>[.dirty]`. Gives traceability + clean/dirty signal.
- `<build-stamp>` — a monotonic per-invocation token that guarantees folder uniqueness even across *dirty rebuilds with no new commit* (where git-describe alone repeats). Two equally-good sources; pick one in §5:
  - a compact UTC timestamp `YYYYMMDDTHHMM` (no state file; clock is the monotonic source), or
  - a gitignored `.build-seq` integer counter (shorter labels, strictly ordered).

Example displayed version: `0.39.2+17.g9dd2d78.dirty.20260528T1408`.

This is semver-legal: everything after `+` is build metadata, ignored for precedence, so it cannot fight the release version or the reagent invariant.

### 3.2 Separate the folder name from the data-dir key

The current design fuses two things that want *opposite* behavior during iteration:

| Concern | Wants | Keyed on |
|---|---|---|
| Extract-folder name on Desktop | **unique per build** (so a running instance never locks the next build's target) | the full build label (incl. build-stamp) |
| Per-instance data dir | **stable across rebuilds of the same work** (so your test session — agents, panes, auth — survives an iterate-rebuild cycle) | the **git branch** (matches `task dev`'s existing `~/.agentmux/dev/<branch>/` precedent) |

Because every build gets a unique folder, `package-portable.sh`'s running-instance lock check *never fires* — the target is always empty. The lock problem dissolves rather than being worked around.

Because the data dir keys on branch (not the per-build label), rebuilding the same patch 10 times reuses one data dir — your smoke session persists across iterations instead of resetting every build. This is the behavior the user actually wants for "iterate on patches."

> Sub-note: branch-keyed local-portable data dirs live under the `dev-portable` channel (`SPEC_DATA_CHANNELS_2026_05_24.md` §2.2) — e.g. `~/.agentmux/dev-portable/<branch>/`. Confirm the channel/path scheme in `data_paths.rs` during implementation; the principle is "key on something stable across rebuilds," and branch is the natural choice already used by dev.

### 3.3 Release path is unchanged

`task release` remains the *only* thing that moves the committed version: consume changesets → bump → update `VERSION_HISTORY.md` → commit `chore: release vX.Y.Z` → (CI) tag `vX.Y.Z`. The git tag is what `git describe` anchors to, so release tagging must exist (verify in §5). Feature branches stay clean: a changeset file, never a version-file edit.

## 4. Why this fixes all three failures

| Failure | Fix |
|---|---|
| Cross-branch collision | No bump at all → no two branches can claim the same version. Folder name carries a unique build-stamp → no folder/data-dir collision regardless of branch. |
| Stranded commit on failure | `task package` never commits → a failed build leaves git pristine. |
| Fights changesets contract | `task package` never touches version files → feature branches stay clean; nothing to revert; reagent invariant never tripped by a smoke build. |

Plus two bonuses: builds become **traceable** (sha in the label → check out exact source) and **honestly flagged** (`.dirty` tells you a ZIP was built from uncommitted changes — no more "is this the fix or the last clean build?").

## 5. Open implementation decisions (resolve at code time, not blocking the pattern)

1. **build-stamp source:** timestamp (zero state) vs `.build-seq` counter (shorter, strictly ordered). *Recommendation:* timestamp — no gitignored state file to manage, and ordering is preserved by wall-clock.
2. **Release tags exist?** `git describe --tags` needs them. Verify CI tags `vX.Y.Z` on release; `--always` is the safe fallback (degrades to bare sha) if a tag is ever missing. If release isn't tagging, add it — it's independently good practice and cheap.
3. **Data-dir key exact path:** confirm `data_paths.rs` derives the `dev-portable` path from branch and that the build can pass the branch through (env var or `git rev-parse --abbrev-ref HEAD` at package time). Mirror the `dev` keying logic so there's one mechanism, not two.
4. **`package:local`:** currently a deprecated alias of `package`. Either delete it or repoint it as the documented name for the ephemeral build (it *was* ephemeral once — this restores that meaning, correctly this time).

## 6. Implementation sketch

`Taskfile.yml` `package`:
```yaml
package:
    desc: 'Build + package a portable to ~/Desktop with an ephemeral, traceable
           build label. Does NOT bump or commit — the release version moves only
           via `task release`. Every build gets a unique folder (no lock), a
           branch-stable data dir (session persists across rebuilds), and a label
           that embeds the git sha + dirty flag for traceability.'
    platforms: [windows]
    env:
        AGENTMUX_BUILD_CHANNEL_DEFAULT: dev-portable
    cmds:
        # NO bump, NO commit. Derive an ephemeral label and export it for the
        # build + package steps to stamp. git describe gives traceability;
        # the timestamp guarantees a unique folder even on dirty rebuilds.
        - cmd: # compute label, export AGENTMUX_BUILD_LABEL for downstream cmds
        - task: build:frontend
        - task: build:backend
        - task: build:host
        - task: bundle
        - bash scripts/package-portable.sh   # reads AGENTMUX_BUILD_LABEL
```

`package-portable.sh`: name the extract folder + ZIP from `AGENTMUX_BUILD_LABEL`; drop the running-instance lock check (or keep it as a cheap belt-and-suspenders — it will simply never trigger).

`data_paths.rs`: for the `dev-portable` channel, key the data dir on branch, not version.

`bump-wrapper.sh`: no longer invoked by `package`; remains the release-flow tool. No change needed beyond removing the package call.

`CLAUDE.md`: rewrite the "Build versioning" section — the current claim "the committed version IS the durable monotonic counter; each build advances it for real" is the broken premise and must go. Replace with: local builds are labeled, not versioned; the release version moves only via `task release`.

## 7. Risk + reversibility

Low risk, high reversibility. The change *removes* a git-mutating step and *adds* a read-only label derivation. Worst case: the label format needs tweaking (cosmetic). The release path is untouched. If anything regresses, reverting restores the auto-bump — but the auto-bump is the thing that keeps breaking, so the bar for the new scheme is "stop breaking," which deletion of the commit step structurally achieves.

The one behavior change a developer will notice: smoke-build ZIPs are named `agentmux-0.39.2+17.g9dd2d78.dirty.20260528T1408-x64-portable` instead of `agentmux-0.39.3-x64-portable`. Longer, but unambiguous and honest. If the length grates, the `.build-seq` counter variant (§5.1) shortens it to `0.39.2+b18`.

## 8. Sources

- [Semantic Versioning 2.0.0 §9–10 (prerelease vs build metadata)](https://semver.org/)
- [Semver: the unknown buildMetadata (dev.to)](https://dev.to/ayc0/semver-the-unknown-parts-271)
- [Proper Release Versioning Goes a Long Way (Memfault Interrupt)](https://interrupt.memfault.com/blog/release-versioning)
- [git-describe documentation (--tags --dirty --always)](https://git-scm.com/docs/git-describe)
- [versioneer (git-describe → PEP 440 build identifiers)](https://pygraphistry.readthedocs.io/en/0.11.6/versioneer.html)
- [Changesets detailed explanation (bump at release, not on branches)](https://github.com/changesets/changesets/blob/main/docs/detailed-explanation.md)
- [Guide to version management with changesets (LogRocket)](https://blog.logrocket.com/version-management-changesets/)
- [Component Versioning — Microsoft Engineering Playbook](https://microsoft.github.io/code-with-engineering-playbook/source-control/component-versioning/)
- [GitVersion — continuous deployment mode (prerelease tags for CI builds)](https://gitversion.net/docs/reference/version-increments)
