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

8. **Check every channel once the release is published and its run is green.** Each check is read-only (a command or a public page), takes a minute and says which way of installing is behind. Set `V=X.Y.Z` first. Report each line to your leader as `ok`, or as `behind` with what you saw, and tell the owner of a channel that is behind: the landing site, the docs deploy and the Store are the platform lead's (AgentY); the Homebrew tap and the apt and dnf repositories are the release owner's, who fixes them with a task and a PR. A channel that is behind is a task, not a reason to cut a new version.
   1. **GitHub Release:** `gh release view vX.Y.Z --json assets --jq '.assets[].name'` lists the installers, the four Headless packages, the install scripts, the envelope with its `.sig` and `.sshsig`, and the manifest; the manifest check in step 7 says `verified`.
   2. **Install scripts on the site:** `curl -s https://agentmux.ai/install.sh | sha256sum` equals `gh release download vX.Y.Z -p install.sh -O - | sha256sum`; the same for `install.ps1`. The site copies them when the landing site ships, which a release run does not do: if they differ, ask the landing owner for a ship.
   3. **Landing page:** `curl -s https://agentmux.ai/release.json` shows `"version": "X.Y.Z"`, with `published_at` equal to the release's. It moves with the same landing ship.
   4. **apt and dnf:** `curl -s -o /dev/null -w '%{http_code}\n' https://packages.agentmux.ai/apt/pool/main/a/agentmux/AgentMux_${V}_amd64.deb` and `.../rpm/x86_64/AgentMux-${V}-1.x86_64.rpm` both answer `200`, and the run's four container checks and its upgrade check passed. These repositories carry amd64 and x86_64 only.
   5. **Container images:** the digest `ghcr.io/agentmuxai/agent-base` serves for `:latest` and for `:X.Y.Z` is the `agent-base` digest in the signed manifest. Take an anonymous pull token from `https://ghcr.io/token?scope=repository:agentmuxai/agent-base:pull`, then `curl -sI` the manifest URL (`/v2/agentmuxai/agent-base/manifests/<tag>`, with the token and an `Accept` header naming the OCI index and manifest types) and read `docker-content-digest`.
   6. **Homebrew:** step 7's last line: `brew info --cask agentmuxai/tap/agentmux` shows `X.Y.Z`.
   7. **Microsoft Store:** the public display catalog names the package: `curl -s 'https://displaycatalog.mp.microsoft.com/v7.0/products/9P9QCXNNCRK3?market=US&languages=en-US'` contains `AgentMux.AgentMux_X.Y.Z.0_x64`. If it does not, the run summary's "Microsoft Store" line says whether this release was submitted. The daily Store listing check says the same thing as an issue, with the version the Store serves. A Store upload made by hand goes into the company record the same day.
   8. **WinGet:** `curl -s -o /dev/null -w '%{http_code}\n' https://api.github.com/repos/microsoft/winget-pkgs/contents/manifests/a/AgentMux/AI/X.Y.Z` answers `200`. While the first submission is still waiting for a moderator it answers `404`: write "WinGet: not live" and do not call it a failure. The run summary's "WinGet" line says whether this release was submitted.
   9. **Docs:** `gh run list --repo agentmuxai/agentmux-docs --workflow deploy.yml --limit 1` shows a deploy that started after the last docs change on main (`gh api repos/agentmuxai/agentmux-docs/commits/main --jq .commit.committer.date`). The docs deploy is started by hand, so an older deploy means the live pages are behind: ask for one.
   10. **Updater, by a person:** from the previous release's build, "Check for updates" offers X.Y.Z. This one needs someone at a spare machine; skip it when there is none and say so.

## Rules

- **Check who else is releasing first** (recent `chore: release` commits on main, `Presence who`), so two release PRs don't collide.
- `scripts/release.sh --as <patch|minor|major>` is the general form; `patch` is the override the operator usually means. Without `--as` the highest pending type wins.
- Never bump versions in a feature PR.
- A release is not finished until its verification list is done: every package published, the update manifest verifies, the Homebrew tap serves the new version (step 7), and each channel in step 8 is checked and reported.
