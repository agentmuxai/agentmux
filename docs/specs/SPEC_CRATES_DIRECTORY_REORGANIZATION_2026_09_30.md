# Move the Rust crates under `crates/`, with a merge freeze and a rebase plan

**Author:** AgentY (narko), at operator request
**Created:** 2026-09-30
**Status:** implemented — spec #4072; the move is PR #4076, rebase-merged 2026-09-30 17:57 UTC (pure-move commit 3f1dd8ce0, path fixes a100543bb). Freeze 16:43–17:57 UTC. Follow-ups: agentmux-docs#140 (site source links, submodule), plus two path updates in private repos (drift reporter, ReAgent triage glob), neither listed in §4.3.
**Scope:** folder layout of `agentmux-*` crates in this repo, and every path that points at them. Package names, binary names and Rust module paths do not change.

## 1. What changes

The six workspace crates move from the repo root into `crates/`, and the folder drops the
`agentmux-` prefix:

| Before | After | Package name (unchanged) | Binary (unchanged) |
|---|---|---|---|
| `agentmux-cef/` | `crates/cef/` | `agentmux-cef` | `agentmux-cef` |
| `agentmux-srv/` | `crates/srv/` | `agentmux-srv` | `agentmux-srv` |
| `agentmux-launcher/` | `crates/launcher/` | `agentmux-launcher` | `agentmux-launcher` |
| `agentmux-mcp/` | `crates/mcp/` | `agentmux-mcp` | `agentmux-mcp` |
| `agentmux-common/` | `crates/common/` | `agentmux-common` | (library) |
| `agentmux-bashwrap/` | `crates/bashwrap/` | `agentmux-bashwrap` | `agentmux-bashwrap` |

The rule for any old path: drop `agentmux-` from the first segment and prefix `crates/`.
`agentmux-srv/src/backend/x.rs` becomes `crates/srv/src/backend/x.rs`.

The top level goes from 23 folders to 18, and the Rust code sits beside `frontend/` as one group.
New crates go under `crates/<short-name>/` with package name `agentmux-<short-name>`.

## 2. Why this shape

- **`crates/` is the common Rust workspace layout** (ripgrep, ruff, wasmtime, bevy).
- **Bare top-level names would collide or confuse:** `cef/` next to `scripts/cef-build/`,
  `common/` with no context, and `build/` already exists.
- **Package and binary names stay `agentmux-*` on purpose:**
  - The binaries are user-visible: Task Manager, firewall prompts, installers, the MSIX manifest,
    code signing, crash dumps, Linux packages. The launcher also spawns them by name.
  - A package named `cef` would clash with our `cef` dependency (the cef-rs binding crate).
  - `agentmux_common::…` imports stay as they are, so no Rust source changes beyond relative paths.
- **Shorter paths** also help Windows `MAX_PATH` (`crates/srv` is 3 characters shorter than
  `agentmux-srv`, and deep ts-rs and CEF paths are where we hit the limit).

## 3. Non-goals

- Renaming packages, binaries, Rust modules, installers, or the `agentmux-docs` crate list.
- Moving `frontend/`, `scripts/`, `docs/` or anything else.
- Rewriting dated records (specs, retros, reports, `VERSION_HISTORY.md`). They were correct when
  written; §1's rule maps any old path. Living docs are updated (§4.4).

## 4. What has to change besides the move

Measured on `origin/main` at `13c858079`.

### 4.1 Paths relative to a crate folder, which gain one `../`

Moving a crate one level deeper breaks every path that climbs out of it. These fail in two
different ways, so each class has its own check.

| What | Count | Failure if missed | How it's caught |
|---|---|---|---|
| ts-rs `#[ts(export, export_to = "../../frontend/types/rpc/")]` in `agentmux-srv` | 374 in 48 files | **Silent:** bindings are written to `crates/frontend/types/rpc/` instead of `frontend/types/rpc/` | `scripts/check-rpc-bindings.sh` (committed bindings must equal generated), plus an explicit check that `crates/frontend/` does not exist after `cargo test` |
| `include_str!` / `include_bytes!` climbing to repo-root files | 8, listed below | Loud: compile error | CI build on all three OSes |
| `env!("CARGO_MANIFEST_DIR")` joins to repo-root paths | 9 uses; 4 climb to the root with one `.parent()` and need two (`agentmux-common/src/api_types.rs`, `agentmux-common/src/redact.rs`, `agentmux-srv/src/backend/reactive/types.rs`, `agentmux-srv/src/backend/rpc_types/block.rs`) | Tests read the wrong folder and fail | Tests; review every hit by hand |
| `agentmux-cef/build.rs` reads the workspace lockfile as `../Cargo.lock` (build scripts run in the crate folder) | 1 | **Silent:** the version panel's CEF row falls back to "unknown" | Check the version panel on a built app; found during implementation |
| `path = "../agentmux-common"` in each crate's `Cargo.toml` | 5 | Loud: cargo error | Any build |
| `agentmux-launcher/build.rs` and `src/tray/windows.rs` pointing at `../agentmux-cef/resources/win/agentmux.ico` | 2 | Wrong or missing exe icon (build.rs); tray icon falls back (runtime) | Windows packaging run; check the exe icon by eye |

The 8 `include_*` sites that leave their crate, each of which gains one `../`:

| Site | Resolves to |
|---|---|
| `agentmux-cef/src/commands/platform.rs:12` | `settings-template.jsonc` |
| `agentmux-cef/src/macos_compat.rs:654` | `assets/linux/icons/hicolor/512x512/apps/agentmux.png` |
| `agentmux-launcher/src/notify/windows.rs:69` | `assets/favicon-150x150.png` |
| `agentmux-launcher/src/tray/linux.rs:138` | `assets/favicon-71x71.png` |
| `agentmux-launcher/src/tray/windows.rs:401` | `assets/favicon-150x150.png` |
| `agentmux-launcher/src/tray/windows.rs:517` | `assets/favicon-150x150.png` |
| `agentmux-srv/src/backend/layout_file/tests.rs:367` | `schema/agentmux-layout.v1.schema.json` |
| `agentmux-srv/src/backend/wconfig/mod.rs:30` | `settings-template.jsonc` |

The other `include_*` calls with `../` resolve inside their own crate (`../../resources/…` from
`src/tray/`) and don't change, because a crate's internal layout doesn't change. Recompute the
list at implementation time rather than trusting this table: resolve each `include_*` path
against its file's folder and keep the ones that land outside the crate.

### 4.2 Build, CI, packaging and tooling

About 45 files outside the crates name a crate folder. The biggest groups:

- **Workspace `Cargo.toml`:** `members = [...]` becomes `crates/*` entries (explicit list, not a glob,
  so a stray folder never joins the workspace).
- **`.github/workflows/`:** `build-windows.yml`, `ci-nightly-build.yml`, `container-image.yml`,
  `srv-image.yml`, and any `--manifest-path`, `working-directory` or cache-key path.
- **`scripts/`:** packaging (`package.sh`, `package-macos.sh`, `package-linux.sh`,
  `build-deb-linux.sh`), CEF verification (`verify-cef-*.sh`, `resolve-cef-runtime*.sh`), and gates
  (`check-name-resolver-callers.sh`, `check-rpc-bindings.sh`, `check-rpc-codegen-hygiene.mjs`,
  `check-muxbus-credential-store.sh`, `check-bundled-tools.sh`, `check-time-helpers.mjs`).
- **Gate allowlists with hard-coded paths:** `scripts/check-time-helpers.allow` (91 entries) and the
  test fixtures in `scripts/docs-stale-sweep.test.mjs` and `scripts/ci-classify-changes.test.mjs`.
  A missed allowlist entry makes a gate fail, which is loud but wastes a CI round.
- **`docker/Dockerfile.agent-agentmux`** and any other `COPY agentmux-*/`.
- **`Taskfile.yml`**, **`.gitignore`**, **`.vscode/`**, **`.claude/`** (skills that cite paths),
  **`tools/tests/lib/instance-discovery.mjs`**.
- **`scripts/ci-classify-changes.mjs`:** today it never classifies a crate change as docs-only
  (its docs-only list doesn't include crate folders). Confirm that still holds for
  `crates/*/docs/**` or any Markdown inside a crate, so Rust tests are never skipped.

The complete list comes from one command, run again after the change to prove it's empty:

```bash
git grep -nE 'agentmux-(cef|srv|launcher|mcp|common|bashwrap)/' -- . \
  ':!docs/specs' ':!docs/retro' ':!docs/reports' ':!docs/analysis' ':!VERSION_HISTORY.md' ':!Cargo.lock'
```

Hits left in comments that describe history are fine; everything else must be gone.

### 4.3 Things outside this repo

- **agentmux-docs:** `scripts/build-rust-docs.mjs` passes package names (`-p agentmux-srv`), which
  don't change. Its `src/agentmux` submodule is pinned to a commit, so nothing moves until it's
  bumped. After the bump, check `/api/rust/` still builds (that deploy now fails loudly, #137).
- **CEF drift reporter (private infrastructure repo):** reads `scripts/cef-build/cef-runtime-pins.sh` and
  `.github/workflows/release.yml`, neither of which moves. No change.
- **Private repos (ReAgent, dev tools, the cloud repo):** grep each for `agentmux-(srv|cef|…)/` paths
  (review prompts, scripts). Fix any hits in follow-up PRs in those repos.
- **Agent instructions:** Global Memory, per-agent `CLAUDE.md`, Personal Memory and skills that
  cite `agentmux-srv/src/...`. They go stale, not wrong in a dangerous way. Grep and fix the
  shared ones; each agent fixes its own personal notes.

### 4.4 Living docs

`README.md`, `BUILD.md`, `CONTRIBUTING.md`, `docs/cef-build/*.md` and any doc with status
`living` or `active` get the new paths. Dated records keep the old ones. Add one line to
`CONTRIBUTING.md`: "crates live in `crates/<name>/`, package `agentmux-<name>`".

## 5. Freeze and rebase plan

The only real risk is in-flight work: branches that edit or add files under the old folders.
Git follows *edits* to moved files through a rebase. It does not move *new* files a branch added
under an old folder; those land in a folder Cargo no longer builds, and the build fails. The plan
keeps the number of such branches near zero and gives everyone a one-command fix for the rest.

### 5.1 Before the freeze (T−24 h to T0)

1. **Announce** to every agent with open work (jekt, plus a pinned issue): freeze time, expected
   length (about 3 hours), and "don't start new branches that add files under `agentmux-*/`".
2. **Drain the queue.** Every approved PR merges on approval before T0. PRs that can't make it
   stay open and rebase afterwards with the helper (§5.4).
3. **List what's left** at T0: open PRs and known agent branches. Each owner is told they'll need
   the helper.

### 5.2 During the freeze (T0 to T+3 h)

- **No merges to `main`** except the move PR. Other PRs can still be opened, pushed and reviewed.
- Enforcement is by announcement. If it's needed, the operator adds a temporary branch rule on
  `main`, which is an admin action (`gh-agent sudo`). It's removed when the move lands.
- **Release work is paused.** No release PRs during the window.

### 5.3 The move PR

One PR, two commits, merged with **rebase-merge** (not squash) so the pure move stays its own
commit:

1. **Commit 1, pure move:** `git mv agentmux-<x> crates/<x>` for all six, nothing else. Every file
   shows as a 100 % rename, so `git log --follow` and `git blame` stay exact.
2. **Commit 2, path fixes:** everything in §4.1, §4.2 and §4.4, and the helper script (§5.4).

Validation before merge (all must pass):

- [ ] PR CI on all three OSes, including `check-rpc-bindings.sh` and every grep gate.
- [ ] `crates/frontend/` does not exist after `cargo test` (the silent ts-rs failure in §4.1).
- [ ] The §4.2 grep returns only historical comments.
- [ ] `ci-nightly-build.yml` and `ci-nightly-artifacts.yml` dispatched on the branch
      (`gh-agent workflow run … --ref <branch>`): Windows installer and MSIX, macOS signed DMG,
      Linux AppImage/deb/rpm all build.
- [ ] Windows `task dev` launches, and the exe and tray icons are right (§4.1 last row).
- [ ] A test rebase of one real open branch with the helper succeeds.

### 5.4 After merge: rebasing a branch

Commit 2 adds `scripts/migrate-branch-to-crates.sh`. Run it on any branch created before the move:

```bash
git fetch origin
git rebase origin/main              # edits to moved files follow the rename
bash scripts/migrate-branch-to-crates.sh   # moves files the branch ADDED under agentmux-*/
```

The helper:

- Finds files the branch added under an old folder
  (`git diff --name-only --diff-filter=A origin/main...HEAD`, filtered to `agentmux-*/`) and
  `git mv`s each to its `crates/` path.
- Rewrites `agentmux-<x>/` paths in files the branch changed, and bumps `../../frontend/types/rpc/`
  in any new ts-rs `export_to` it finds.
- Prints what it did and exits non-zero if anything is left under an old folder, so a
  half-migrated branch can't go green by accident.
- Changes nothing on a branch with no hits, so it's safe to run everywhere.

If a rebase hits conflicts, `git rebase --abort` and rebase onto the commit *before* the move
first (`git rebase <move-sha>~1`), then onto `main`. That separates "your conflicts" from "the
move".

Worktrees and uncommitted changes: commit or stash first. A stash applied after the rebase puts
edits to moved files in the right place only for files that exist in both; new files from a
stash need the helper too.

Build caches: the first build after the move recompiles everything, including the CEF wrapper
(minutes, not hours). `target/` doesn't need deleting.

### 5.5 After merge: announce and follow up

1. Jekt every agent: the move has landed, the §5.4 commands, and "no new files under `agentmux-*/`".
2. Lift the freeze (and remove the temporary branch rule, if one was added).
3. Bump agentmux-docs' submodule and confirm `/api/rust/` still builds.
4. Fix the shared agent instructions (§4.3) and file follow-ups for sibling repos.
5. Check the next nightly and the next release run.

## 6. Rollback

Until anything else merges on top, revert the move PR (both commits) and re-announce. After other
PRs merge on top of it, roll forward instead: fix whatever broke in a new PR. Reverting a move
under other people's work would make them migrate twice.

## 7. Cost and timing

| Step | Effort |
|---|---|
| Commit 1 (move) | minutes |
| Commit 2 (§4 fixes, helper script, docs) | about half a day |
| Validation (PR CI, dispatched nightly build and packaging) | about 1.5 hours of CI, mostly waiting |
| Freeze window | about 3 hours from T0 to merge |
| Each rebasing branch | one command, plus conflicts the branch would have had anyway |

Best time: right after a release, when the queue is short. v0.59.0 shipped 2026-09-30, so the
window between now and the next release is the one to use.

## 8. Open decisions for the operator

1. **The freeze window** (T0). Everything else in this spec can proceed on the go-ahead.
2. **Enforce the freeze with a temporary branch rule, or by announcement only?** Recommendation:
   announcement only. The team is small and responsive, and a rule is one more thing to forget to
   remove.
