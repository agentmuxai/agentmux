---
name: release
description: Cut an AgentMux release PR with scripts/release.sh, including the operator's "patch override" (a release bumped as a patch whatever the pending changesets ask for). Use when asked to release, bump the version, "bump the patch override", or "do a patch override release".
---

# Releasing AgentMux, and the "patch override"

Feature PRs never bump versions: a **release PR** owns every version file. This skill is how to make one.

## What "patch override" means

When the operator says **"bump the patch override"**, **"do a patch override release"** or similar, they mean a release PR bumped as a **patch**, overriding the default bump type, so a build with the merged work exists to test or ship.

The release step picks the HIGHEST type among the pending `.changesets/*.md` (major > minor > patch), so one `minor` changeset makes the whole release a minor bump. The override forces a patch whatever they say.

## Steps

In a clone of `agentmuxai/agentmux`, from a fresh branch off up-to-date `origin/main`, named `<agent>/release-patch-<version>`:

1. **Wait until the PRs that should be in the build have merged.** The release consumes EVERY pending changeset on main at that moment, including other agents' merged work.
2. `scripts/release.sh --dry-run --as patch` shows the bump and the entries. It warns when the override is lower than the changesets ask for; that is expected, not an error.
3. `scripts/release.sh --as patch` bumps `package.json`, `Cargo.toml` and the lockfiles, appends `VERSION_HISTORY.md`, deletes the consumed changesets and stages everything.
4. Commit as `chore: release vX.Y.Z` and open a PR (title prefix `<name>@<host>: `, the agent tag in the body). It needs no changeset of its own: release PRs are exempt.
5. **Merge only on muxreview's approval for the head commit with CI green**, like any PR. The release workflow builds and tags it.

## Rules

- **Check who else is releasing first** (recent `chore: release` commits on main, `Presence who`), so two release PRs don't collide.
- `scripts/release.sh --as <patch|minor|major>` is the general form; `patch` is the override the operator usually means. Without `--as` the highest pending type wins.
- Never bump versions in a feature PR.
