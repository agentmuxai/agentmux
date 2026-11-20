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
6. **The release publishes only when every package job has succeeded**: the desktop installers and the Headless server package for each platform. If one fails (a registry or network error, say), nothing is published and the run shows the failed job. Use "Re-run failed jobs" on the run (`gh-agent run rerun <run id> --failed`); the release publishes when all of them pass. The tag stays, so don't cut a new version for it. A failure that comes back is a CI problem to fix in a PR, not a reason to publish without the package.
7. **Bump the Homebrew tap, once the release is published** (it is part of the release: people who install with `brew` get nothing new until it is done). The tap's workflows do not open the pull request for you: GitHub Actions is deliberately not allowed to create pull requests there, because the same permission would let a workflow on any branch approve one. A stale check opens an issue in the tap when the cask falls behind, which means this step was missed.
   1. Clone `agentmuxai/homebrew-tap` (inside your workspace) and branch `bump-X.Y.Z` off its `main`.
   2. Verify the release's signed manifest first: download `AgentMux-X.Y.Z-manifest.json`, the envelope, `.sig` and `.sshsig` from the release into an empty folder and run `node scripts/release-manifest.mjs verify --dir <folder> --tag vX.Y.Z --id k1 --keys-file crates/common/update-keys.json` from this repo; it must say `verified`.
   3. In the tap run `GH=gh-agent scripts/bump.sh X.Y.Z` (plain `gh` for a person). It renders `Casks/agentmux.rb` from that same signed manifest and changes nothing else.
   4. Commit `Casks/agentmux.rb`, open the PR with `gh-agent` (title prefix `<name>@<host>: `, the agent tag in the body), and merge on review with its three checks (`scripts`, `cask`, `provenance`) green. Never turn on the Actions "create and approve pull requests" setting to automate this, and never put a token or App key in the tap.
   5. Check that `brew` reads the new version, read-only: `brew update && brew info --cask agentmuxai/tap/agentmux` shows `X.Y.Z` (do not install over a running AgentMux).

## Rules

- **Check who else is releasing first** (recent `chore: release` commits on main, `Presence who`), so two release PRs don't collide.
- `scripts/release.sh --as <patch|minor|major>` is the general form; `patch` is the override the operator usually means. Without `--as` the highest pending type wins.
- Never bump versions in a feature PR.
- A release is not finished until its verification list is done: every package published, the update manifest verifies, and the Homebrew tap serves the new version (step 7).
