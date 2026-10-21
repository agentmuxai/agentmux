# ANALYSIS evidence: Partial audit: scripts, task runners, CI workflows (snapshot origin/main v0.59.35, 2026-10-20)

**Date:** 2026-10-20
**Status:** analysis. The evidence behind docs/analysis/ANALYSIS_DEAD_CODE_AND_TREE_SHAKING_2026_10_20.md (read-only, static: nothing was compiled or run). Re-verify an item before acting on it.
**Note:** where this file mentions `scratch/...` helper scripts or listings (`deps.py`, `allows.py`, `allowed.txt`), those were throwaway analysis files and are not kept; the lists are reproducible with ripgrep and `knip` as the text describes.


Scope: `scripts/**`, `tools/**`, `build/`, `packaging/`, `docker/`, `.githooks/`, `.config/`, `Taskfile.yml` (only one; no includes), `package.json` scripts, `.github/workflows/*.yml`, `.github/actions/**`.
Snapshot read-only at a read-only extract of origin/main. No cargo, no repo scripts run, no network.

## Method

- Corpus: every non-binary file in the snapshot minus `node_modules`, `package-lock.json`, `Cargo.lock` (6702 files). 358 target files (scripts/tools/build/packaging/docker/.githooks/.config/.github).
- For every target I ran `rg -l -F <basename>` over the corpus (hidden files included), removed self, and split hits into: code/CI refs (Taskfile, `.github`, package.json, other scripts, crates, frontend, vite config, packaging), the file's own `*.test.*`, and doc refs (`docs/**` and any `*.md`). The full matrix is in `scratch/refs.txt` (tab separated: path, docRefCount, codeRefs, ownTests).
- Dynamic references checked by hand: `scripts/build-${f}-linux.sh` (package-linux.sh:131, so build-appimage/deb/rpm/tarball-linux.sh are live), `core.hooksPath .githooks` (package.json `prepare`), `uses: ./.github/actions/actions-broker`, `uses: ./.github/workflows/build-*.yml`, vitest default include (see section 2), nextest auto-discovery of `.config/nextest.toml`.
- Path check: every `scripts|tools|packaging|docker/...` path named in workflows, Taskfile, package.json, knip, vitest/vite configs and `.githooks` exists (the only "missing" hits were build outputs, `docker/*-action` action names and globs). Every path named inside `scripts/` and `tools/` was also checked; the real dangling ones are in section 6.
- Caveats: basename greps over-match for generic names (`env.mjs`, `capture.mjs`, `settings.mjs`, `README.md`); I read the hits by hand for every item reported. A "code ref" that is only a comment is called out as "comment only". Things consumed outside this snapshot (private release monitor, `repository_dispatch` senders, agentmux-landing, cloud repos) cannot be seen here.

Legend: CONFIRMED = zero references of the kind stated, from the greps listed. LIKELY = needs a human check (external use, intent, or judgment).

---

## 1. Dead or stale scripts and tools

### 1.1 `scripts/verify-package.sh` (163 lines, 4 KB) - CONFIRMED dead
- Claim: verifies an Electron/Wave package (`electron-builder`, `MacOS/Wave`, `app.asar.unpacked`, `agentmuxsrv*`, `wsh-${VERSION}-*`); none of that exists any more (Electron/Tauri and wsh retired).
- Evidence: `rg -F verify-package.sh` hits only 3 docs (`docs/reports/REPORT_REPO_HEALTH_AUDIT_2026_07_05.md`, `..._07_20.md`, `docs/specs/archive/rebrand.md`); no hit in Taskfile, `.github`, package.json, scripts, crates. 21 `electron|wave|wsh` matches inside it.
- Action: delete. Risk: none (the health-audit reports already flagged it).

### 1.2 `scripts/benchmarks/measure-performance.ps1` (220) and `.sh` (223) - CONFIRMED dead (documented, but documents a dead layout)
- Claim: WaveMux/Tauri-era installer-size and startup benchmark. Paths `src-tauri\target\release\WaveMux.exe`, `wavemux`, MSI/NSIS bundle, "electron vs tauri" reduction math.
- Evidence: `rg -F measure-performance` hits only `scripts/benchmarks/README.md` (documents them) and `docs/analysis/full-codebase-audit-2026-04-03.md` (which already said "update paths from src-tauri"). Zero hits in Taskfile/CI/package.json.
- Action: delete both scripts and the matching README sections (README is 254 lines and also documents the live `agent-open.mjs`). Risk: low.
- Not dead: `scripts/benchmarks/agent-open.mjs` (245) + test. Only its own test and 2 docs reference it, so it is a documented manual tool (README) - keep. Its test runs under vitest.

### 1.3 `scripts/dev-tools/*.ps1` (10 files, 381 lines, 14 KB) - LIKELY dead
- Files: clear-console, click-console, click-console-area, devtools-console, open-devtools, reload-app, screenshot, screenshot-crop, scroll-console, scroll-console-top.
- Claim: SendKeys/win32 click hacks to drive a DevTools console and 1920x1080 `CopyFromScreen` screenshots; hard-coded process name `agentmux`; superseded by CDP (`scripts/ui-screenshots/*`, `verify-shortcuts*`) and the MuxAPI `UI` tool. No README in the directory.
- Evidence: `rg -F` of each basename = 0 code refs; `rg 'dev-tools'` hits only `tools/sandbox/*` (unrelated "install-dev-tools") and generic prose docs. 6 of the 10 have one doc mention each (retro/status notes).
- Action: delete the directory after one human glance. Risk: low (a human may keep one on a personal machine).

### 1.4 `tools/sandbox/` (12 files, 2427 lines, 68 KB) - LIKELY dead
- Claim: WaveMux-era remote "Parsec sandbox host" provisioning (PowerShell). Installs Go/Zig/Task, clones `wavemux`, package name `@a5af/sandbox` / `@agentmuxai/sandbox`, about 140 `wavemux|waveterm` matches across the .ps1 files. Own Pester tests.
- Evidence: `rg -F -e tools/sandbox -e sandbox-health -e setup-sandbox-host -e @agentmuxai/sandbox` outside `tools/sandbox/**` = 0 hits (only `docs/specs/archive/rebrand.md` has an unchecked TODO to review it). Not wired into Taskfile, CI or knip entries.
- Action: delete after the owner confirms nobody provisions sandboxes with it; otherwise rename the WaveMux references. Risk: low-medium (a human may use it manually on a Windows host).
- Related: `tools/sandbox/tests/Sandbox.Tests.ps1` (Pester) is never run by anything (see 2.2).

### 1.5 `tools/enable-crash-dumps.reg` (25 lines) - LIKELY superseded
- Claim: registers WER LocalDumps for the exact name `agentmux-srv.exe`. The shipped srv is `agentmux-srv-{version}-windows.x64.exe` (Taskfile `build:backend:rust:windows`), so the .reg silently matches nothing. `scripts/register-crash-dumps.ps1` was written for exactly that reason.
- Evidence: `rg -F enable-crash-dumps` -> only `docs/archive/CRASH-DUMP-ANALYSIS-2026-03-26.md` and `docs/specs/SPEC_FLEETWIDE_CRASH_HANG_FORENSICS_2026_08_14.md` (which cites both). 0 code refs.
- Action: delete the .reg (keep the .ps1, a documented manual tool with 2 spec refs). Risk: low.

### 1.6 `scripts/import-agents.sh` (530 lines, 27 KB) - LIKELY superseded
- Claim: copies user agent definitions between per-version `objects.db` files and rehydrates the Claude session. Since cross-channel agent persistence (global definition store), `crates/srv/src/registry/def_migrate.rs` does the definition backfill in-process; that Rust file only mentions the script in a comment ("Resilience mirrors ..."). `docs/specs/SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md` says `rehydrate_claude_session` still exists only in this script (Parts B/C not started), so keep until that is decided.
- Evidence: `rg -F import-agents` code hits = `def_migrate.rs` (comment) only; docs = 8 (specs/analysis/VERSION_HISTORY).
- Action: human decision; if rehydrate is dropped, delete. Risk: medium (largest script in this list, only copy of the rehydrate logic).

### 1.7 `scripts/wipe-old-data-dirs.sh` (236 lines, 8 KB) - LIKELY one-off
- Claim: "torch legacy data on this single-user dev machine" (Tauri-era `ai.agentmux.app.*`, `com.a5af.agentmux*`, old `~/.agentmux/<version>/`), written for the 2026-05 data-dir unification, which is archived/superseded.
- Evidence: 0 code refs; docs refs are archive/specs (6). `docs/analysis/ANALYSIS_SCRIPTS_CLEANUP_2026-06-18.md` explicitly kept it as a "standalone utility".
- Action: delete or move to docs/archive. Risk: low.

### 1.8 `scripts/migrate-branch-to-crates.sh` (124 lines, 5 KB) - LIKELY one-off
- Claim: helps rebase branches made before the 2026-09-30 `crates/` move; only useful while such branches exist (3 weeks old).
- Evidence: `rg -F migrate-branch-to-crates` -> only `docs/specs/SPEC_CRATES_DIRECTORY_REORGANIZATION_2026_09_30.md`.
- Action: delete in a month. Risk: none.

### 1.9 `scripts/package-agent.cmd` (19 lines) - LIKELY orphan
- Claim: bridge to run `task package` from an MCP shell; its header cites "CLAUDE.md's 'Launching task package from an agent' section" but there is no CLAUDE.md in the repo (`scripts/check-claude-md-references.sh` enforces its removal). Only one retro doc references it. Its sibling `scripts/dev-agent.cmd` (126 lines) IS referenced (`package-agent.cmd`, `bootstrap.ps1`, 17 docs) - keep.
- Evidence: `rg -F package-agent.cmd` -> 1 doc (`docs/retro/retro-task-package-mcp-timeout-...`). 0 code.
- Action: human check, then delete or document in BUILD.md. Risk: low.

### 1.10 `scripts/test-app-api.mjs` (125 lines) and `scripts/test-splash.sh` (72 lines) - LIKELY ad hoc
- `test-app-api.mjs`: "Ad-hoc MuxAPI smoke test ... as AgentX", default port 61018; only `docs/specs/SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28.md` mentions it. The name looks like a test but nothing runs it (no `.test.` infix so vitest ignores it).
- `test-splash.sh`: manual launcher splash check; one analysis doc (`ANALYSIS_SCRIPTS_CLEANUP_2026-06-18.md`, which kept it).
- Action: delete or move to `scripts/dev/`. Risk: low.

### 1.11 Dead branch inside a live script: `scripts/bump-wrapper.sh` portable-bump auto-append (about 55 of 163 lines)
- Claim: the `-m "build"` / `IS_PORTABLE_BUMP` path (detection near lines 36-52, append block lines 112-163) exists for `task package` bumping the version. `scripts/package.sh` now says "does NOT bump the version and does NOT touch git" and Taskfile `package` has no bump; `release.sh` calls the wrapper without `-m build`.
- Evidence: `rg -n 'bump-wrapper' scripts/package.sh scripts/package-portable.sh Taskfile.yml` = 0 hits. The wrapper itself is live (called by `scripts/release.sh:137+`).
- Action: delete the portable-bump block. Risk: low. Also `scripts/dev-local.sh:13` comment cites nonexistent `scripts/package-local.sh`.

### 1.12 Doc-only (referenced only from docs/specs, comments, or each other) - documented manual tools, probably keep
No Taskfile/CI/package.json entry for these. Plausibly still useful; listed so the stale-docs check can look at them.
- `scripts/ui-screenshots/{cdp-eval,cpu-profile,synthetic-typing-loaf,trace-capture,typing-under-load-experiment}.mjs` (586 lines, 41 KB): CDP profiling one-offs from the 2026-09-23 agent-pane churn investigation (usage in headers; referenced only by `ANALYSIS_AGENT_PANE_FLUSH_REMOUNT_CHURN_2026_09_23.md`, two window-paint analyses, and `SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md`). LIKELY investigation leftovers; `typing-under-load-experiment.mjs` is literally named an experiment.
- `scripts/perf/macos-paint/*` (13 files, 501 lines, 38 KB, no README): macOS paint probes from #4615 / `ANALYSIS_MACOS_WINDOW_PAINT_2026_10_10.md`. Nothing outside the directory imports them (my first basename grep for `env.mjs` was a name collision with other files; verified by path). LIKELY one-off investigation tools.
- `scripts/tab-switch-report.mjs` (94): one comment ref in `frontend/app/store/tab-reveal.ts` + one analysis doc.
- `scripts/docs-status-origin-survey.mjs` (148): referenced only in a comment of `check-new-spec-status.mjs` and in `docs-stale-sweep.mjs` text.
- `scripts/verify-shortcuts.mjs`, `verify-shortcuts-cdp.mjs`, `verify-shortcuts-l3-linux.mjs`, `verify-shortcuts-temp-tree.mjs`, `gnome-grabs.mjs`, `mac-grabs.mjs`: a coherent family (L1/L2 CDP check, shared CDP lib, Linux L3, temp-tree helper, OS grab reports) described by `docs/specs/PLAN_SHORTCUTS_VERIFIED_AND_APP_API_2026_10_10.md`; only the spec, comments in `frontend/app/view/files/files-view.tsx` and `frontend/app/init/pool.ts`, and each other reference them. Not duplicates. Two have vitest tests that do run.
- `scripts/link-md-paths.mjs` (176): self-documented writer, noted in two plans, imported nowhere (`check-doc-links.mjs` only names it in a comment).
- `scripts/check-long-jump.sh` (66): upgrade dogfood tool documented in `SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` s.3.
- `scripts/register-crash-dumps.ps1` (76): documented in 2 specs.
- `scripts/cli-probe/probe.mjs` (186): manual, safe CLI probe (SPEC_RUNTIME_MENU_REMAINING_GAPS s.7); its sibling tests run in CI (`ci-pr.yml:753`).
- `tools/tests/*` (5270 lines, 228 KB): ad-hoc PowerShell/Node harnesses against a running dev instance, documented in `tools/tests/README.md`. In the README: authfile, bench-term-echo, bench-agent-keystroke, bench-term-cross-pane, pane-load, term-keyrepeat-hiccups, vim-freeze-probe, layout/pane-focus/three-pane. NOT in the README: `agent-layout-drift.mjs`, `bench-aggregate.mjs`, `bench-markdown-parse.mjs` (comment ref in `frontend/app/element/markdown-render-work.test.tsx`), `dom-smoke.ps1`. Documented manual tools, not dead.
- `scripts/benchmarks/agent-open.mjs`, `scripts/cef-build/configure-cef-build{.sh,-windows.ps1}`, `scripts/cef-verify.sh`, `scripts/cef-verify-patches.sh`, `packaging/msix/generate-assets.ps1`: documented in `docs/cef-build/*`, `CEF_FORK_MAINTENANCE.md`, or their own headers. Not dead.
- `scripts/gen-seed.js`: regeneration tool for `crates/srv/agent-seed.json` (`ANALYSIS_SCRIPTS_CLEANUP` kept it deliberately); only comments + `ci-classify-changes.mjs` path list reference it. Keep.
- `scripts/dev/gen-splash-font.py` (49): regenerates `crates/launcher/src/splash_font.rs` (the generated file's header names it). Keep.

### 1.13 Root-level configs touching scripts
- `knip.json` / `knip.prod.json` (repo root, adjacent to scope): CONFIRMED nothing invokes `knip` (`rg -w knip` over Taskfile, package.json scripts, `.github`, scripts = 0; only the devDependency and prose). `VERSION_HISTORY.md:581` says npm deps are gated in CI with knip, but no workflow does. `knip.prod.json` has zero references of any kind. LIKELY dead config or a missing gate - needs a human decision either way.
- `.githooks/pre-commit` comment says `npm run setup-hooks`; no such script exists (package.json `prepare` sets `core.hooksPath` instead). Cosmetic.

---

## 2. Tests that nothing runs

### 2.1 What does run (CONFIRMED)
- `vitest.config.ts` `node` project has no `include`, only `exclude: [node_modules, dist, .claude, worktrees, **/tools/**, ...]`, so vitest's default glob picks up every `scripts/**/*.test.mjs` (53 files). CI runs `npx vitest run --shard=N/2` (`ci-pr.yml:848`) and `npx vitest run` nightly (`ci-nightly-build.yml:258`), plus an explicit run of 4 test files on the docs-index matrix (`ci-pr.yml:753`).
- `.test.sh` run in CI (`ci-pr.yml:660-702`): cef-verify, vite-port, prune-dev-builds, cef-build/verify-cef-runtime-windows, verify-cef-framework-darwin, check-hash-from-stdin, check-anonymous-pull. `cef-verify.test.sh` also nightly.

### 2.2 Not run by anything (CONFIRMED by grep for basename, for `*.test.sh` globs and for `for ... in scripts` loops; none found)
| File | Lines | Note |
|---|---|---|
| `scripts/linux-apprun.test.sh` | 149 | tests the extract-once cache in `linux-apprun.sh` (ships in AppImage/deb/tarball) |
| `scripts/stage-linux-runtime.test.sh` | 94 | tests source-map policy in `stage-linux-runtime.sh`; only mentioned in a comment of `package-macos.test.sh` |
| `scripts/package-macos.test.sh` | 140 | source-map policy + srv signing; no workflow, no Taskfile |
| `scripts/verify-angle-libs.test.sh` | 98 | tests `verify-angle-libs.sh`, which Taskfile runs in 3 bundle tasks |
| `tools/tests/lib/bench-stats.test.mjs`, `instance-discovery.test.mjs` | 208 | `node:test` files; header says `node --test tools/tests/lib/...` but vitest excludes `**/tools/**` and no workflow/Taskfile runs `node --test` (`rg -n 'node --test' .github Taskfile.yml package.json` = 0) |
| `tools/sandbox/tests/Sandbox.Tests.ps1` | 164 | Pester; only `tools/sandbox/package.json` `test` script (see 1.4) |
- Suggested action: add the four `.test.sh` (about 480 lines) to the `docs` job list in `ci-pr.yml` (cheap, Linux-only) or delete them; add a `node --test tools/tests/lib` step or move those two tests under `scripts/` so vitest runs them. Risk: low (the shell tests may need a platform quirk fix on first run - not verified here).

---

## 3. Duplicate / overlapping items

| # | Items | Verdict | Referenced one |
|---|---|---|---|
| 3.1 | `scripts/cli-contract/fake-anthropic.mjs` (104) vs `scripts/cli-probe/fake-anthropic.mjs` (155) | LIKELY mergeable: two independent fake Anthropic Messages servers (contract: scripted Bash tool call; probe: canned streaming reply, records model). Different exports, not drop-in. | contract copy by `cli-hook-contract.yml` + two contract scripts; probe copy by `probe.mjs` + `fake-anthropic.test.mjs` |
| 3.2 | `scripts/gen-docs-index.sh` (32) vs `scripts/gen-docs-index.mjs` (691) | The .sh is a thin wrapper; both referenced (Taskfile `docs:index` calls the .sh; CI calls both, step "bash wrapper reaches the same generator"). Redundant but pinned by a CI step. Low value to remove. | both |
| 3.3 | `tools/enable-crash-dumps.reg` vs `scripts/register-crash-dumps.ps1` | LIKELY superseded, see 1.5 | .ps1 (docs) |
| 3.4 | `.github/workflows/input-handler-guardrails.yml` job `layout-reads` | CONFIRMED triplicated: `tools/lint/check-input-handler-layout-reads.sh` runs in `ci-pr.yml:705`, `ci-nightly-build.yml:323` and here, plus Taskfile `check:input-handler-layout` and a `build:frontend` dependency. `sync-ipc` (`check-input-handler-sync-ipc.sh`) is run ONLY by this nightly workflow. The header ("CI runs NIGHTLY, not per-PR ... 2026-06-23") is stale: per-PR CI exists. Action: drop the `layout-reads` job, decide whether `sync-ipc` moves into `ci-pr.yml`. Risk: low. | all |
| 3.5 | `ci-nightly-build.yml` job `gates` mirrors the `docs` job of `ci-pr.yml` | By design ("PR-parity gates"); no action, noted for completeness. | both |
| 3.6 | `scripts/package*.sh` family | Not duplicates, one reachable chain: Taskfile -> `package.sh` -> `package-portable.sh` (Windows); `package-linux.sh` -> `build-{appimage,deb,rpm,tarball}-linux.sh` (dynamic path); `package-macos.sh`; `package-installer.ps1`, `package-msix.ps1`. Only orphan is `package-agent.cmd` (1.9). | - |
| 3.7 | `resolve-cef-runtime.sh` / `-darwin.sh` / `-windows.ps1` | Per-OS siblings, all referenced from Taskfile `bundle:*`. Not duplicates. | all |
| 3.8 | `vite-build.sh` (28) and `vite-port.sh` (113) | Different jobs, both used by Taskfile (`build:frontend*`, `dev:serve`); vite-port also by `crates/cef/src/client/navigation.rs`. Keep. | both |
| 3.9 | `verify-cef-patch.sh`, `verify-cef-version.sh`, `cef-verify.sh`, `cef-verify-patches.sh`, `verify-angle-libs.sh`, `verify-cef-framework-darwin.sh` | Similar names, distinct purposes (runtime vs fork build); all referenced. | all |
| 3.10 | `Taskfile.yml` `dev:serve` (about 420 lines, 881-1300) vs `dev:standalone:serve` (about 80 lines, 1301-1383) | LIKELY duplicated Vite-start / dev-dir logic; refactor candidate only. | both |

---

## 4. Taskfile.yml (1453 lines, 73 tasks) and package.json scripts

Method: every task name extracted; counted `task: name`, `- name` deps, `task name` in other code (excluding docs), and docs.

### 4.1 Broken
- `storybook`, `storybook:build` (lines 98-110) - CONFIRMED broken. They run `npm run storybook` / `npm run build-storybook`; package.json has no such scripts, no storybook dependency, no `.storybook/`, no `*.stories.*` file in the tree (`rg -i storybook package.json knip.json` = 0). `CONTRIBUTING.md:52-55` still tells contributors to run `task storybook`. Action: delete both tasks and the CONTRIBUTING paragraph. Risk: none.
- `package.json` script `"package:portable": "task package:portable"` - CONFIRMED broken: no task `package:portable` exists (tasks are `package`, `package:release`, ...). Action: delete or point to `task package`. The other task-calling package.json scripts (`dev`, `build`, `build:backend`, `package`, `clean`) map to existing tasks.
- `dev:cleardata:windows` (line 445): `cmd: '{{.RMRF}} %LOCALAPPDATA%\agentmux-dev\Data'` - `%VAR%` is not expanded by the shell Task uses and `Remove-Item ... SilentlyContinue` hides the failure. LIKELY a silent no-op; also targets the `agentmux-dev` dir name. Check before relying on it.

### 4.2 Dead legacy (Wave-era release plumbing)
- `artifacts:upload`, `artifacts:download:*`, `artifacts:publish:*`, `artifacts:snap:publish:*`, `artifacts:winget:publish:*` (lines 370-426, 57 lines) plus top vars `ARTIFACTS_BUCKET`, `RELEASES_BUCKET`, `WINGET_PACKAGE` (lines 20-26) - CONFIRMED unreferenced: `rg -n 'artifacts:|ARTIFACTS_BUCKET|RELEASES_BUCKET|WINGET_PACKAGE|snapcraft|wingetcreate'` outside Taskfile hits only archived release docs; `release.yml` header says the workflow "never touches S3/AWS"; `artifacts:upload` copies from `target/release/` with `builder-*.yml` excludes; `artifacts:winget:publish` uses `.UP_VERSION` outside `vars` in `status:`, `.GITHUB_TOKEN`, an `.msi` URL. `docs/specs/CLEANUP_LEGACY_REMNANTS.md:94` and `docs/analysis/full-codebase-audit-2026-04-03.md:163` already flagged them. Action: delete tasks and vars. Risk: low (winget is the only one with a live intent: `release.yml` Phase 4 is commented out, see 5.1).
- `copyfiles:*:*` (line 471): `internal: true`, CONFIRMED zero invocations (`rg -n copyfiles -g '!node_modules'` outside its definition = 0). Delete.
- `build:frontend:dev` (line 302): CONFIRMED zero references (no dep, no CI, no docs). LIKELY dead since `dev:serve` starts Vite itself.
- `outdated`, `version`, `dev:clearconfig`, `dev:cleardata`, `run`, `clean:host`, `clean:cef`, `check:name-resolver-callers`, `check:doc-status`, `check:spec-citations`, `check:no-transition-all`, `docs:index`: no references outside Taskfile (some have 1-4 doc mentions). They are human entry points, not dead; the `check:*` ones duplicate CI invocations of the underlying scripts (CI calls scripts directly, never `task`). `run` (lines 1383-1400) hard-codes `http://localhost:5173`, which `scripts/vite-port.sh` no longer guarantees - LIKELY stale.
- `package:local` is a documented alias of `package` ("kept for muscle memory"); only docs reference it. Keep or delete by taste.

### 4.3 Referenced scripts all exist
Every `bash scripts/...`, `pwsh -File scripts/...`, `sh scripts/...` and `tools/lint/...` call in Taskfile resolves to a file in the snapshot. No commented-out tasks other than the two "removed" notes (`agentmux-wsh`, `dev:installwsh`, comment-only).

---

## 5. GitHub workflows (25 files, plus 1 composite action)

### 5.1 Commented-out code
- `release.yml` lines 426-513 (about 88 lines): "Phase 4 ... DISABLED 2026-08-12" winget and ms-store jobs fully commented out; header (lines 10-17) explains. Intentional parking, but the repo still carries the supporting Taskfile `artifacts:winget:publish` and `WINGET_PACKAGE` var (4.2) and `docs/specs/SPEC_WINGET_PACKAGE_BOOTSTRAP_2026_09_10.md`. Action: keep until the winget bootstrap decision, else delete.

### 5.2 Unconsumed outputs
- `ci-pr.yml` job `changes` outputs `docs_only` (line 137) and `version_only` (line 139) are CONFIRMED never read: `rg -n 'docs_only|version_only' .github scripts/ci-required-check.mjs` shows only the declaration and the two fallback `printf` lines. `ci-classify-changes.mjs` computes and tests both. (`srv-image.yml` has its own classify step and does consume `version_only`.) Action: drop both outputs from ci-pr (or use them). Risk: low.
- All other job outputs (`rust`, `frontend`, `docs_index`, `queue`, `rust_os`, `frontend_shards`, `docs_index_os`, `release.verify.version`, `cef-runtime-pins.*-tag`, `check-macos.available`) are consumed.

### 5.3 Triggers and reachability (all 25 checked)
- No `if: false`, no `workflow_call` workflow without a caller: `build-windows/macos/linux.yml` are called by `release.yml` (lines 245/264/303) and `ci-nightly-artifacts.yml` (42/53/64). Composite `.github/actions/actions-broker` is used twice by `reagent-review.yml` (97, 141).
- Manual-only (`workflow_dispatch` only): `nightly-release.yml` (documented manual fallback, keep), `memory-soak.yml` (see 5.4).
- `build-{windows,macos,linux}.yml` also declare `repository_dispatch` types `build-windows|macos|linux`; no in-repo sender (`rg 'dispatches|repository_dispatch|gh workflow run'` shows only the landing-site dispatch in `release.yml:314`). LIKELY external senders (cloud / release monitor) - cannot check here.
- `reagent-review.yml` has name `muxreview review` and `reagent-review-log-guard.yml` listens for both `"muxreview review"` and legacy `"ReAgent review"`; file names are stale vs the name. Cosmetic.
- `codex-review-gate-sweep.yml` and `merge-queue-codex-status.yml` were split out of `codex-review-gate.yml` on 2026-10-19 (startup_failure); nothing stale.

### 5.4 Overlap
- `memory-soak.yml` ("Memory soak (nightly, Windows)", 150 lines): name says nightly but it has no schedule (header: "No schedule: the nightly check is perf-regression.yml"). `perf-regression.yml` (320 lines) runs the same lab on a schedule. LIKELY keep as a manual single-build run, but rename it. `scripts/perf/memory-lab/gate.mjs` and `spread.mjs` are reachable only through these two workflows.
- `input-handler-guardrails.yml` - see 3.4.

### 5.5 `paths:` filters
- `srv-image.yml` has `'*/Cargo.toml'`; GitHub `*` does not cross `/` and no top-level directory has a Cargo.toml (crates live in `crates/*`), so it matches nothing (CONFIRMED by `ls */Cargo.toml`). Covered anyway by `Cargo.toml`, `Cargo.lock` and the explicit `crates/...` entries. Action: delete the line.
- All other `paths:` entries (`cli-hook-contract.yml`, `release-consistency.yml`, remaining `srv-image.yml` entries, `.dockerignore`, `.cargo/**`) point at existing files/dirs. No workflow references a branch that does not exist (`main` only; `perf-data` is created by `perf-regression.yml`).

### 5.6 Pinned actions
- No unused action pins. Counts: checkout@v4 x34 (one SHA-pinned), setup-node x18, upload-artifact x13, rust-cache x9, rust-toolchain x9, create-github-app-token x5, download-artifact x4, cache x4, setup-python x3, docker build/qemu/buildx/login/metadata (container-image + srv-image), lychee-action (SHA pin), taiki-e/install-action x2, setup-mold x1. All used.

### 5.7 Build workflows vs nightly vs release
- No copy-paste duplication of the three platform builds: `build-*.yml` are reusable and both `release.yml` and `ci-nightly-artifacts.yml` call them. Remaining duplication is the intentional PR-parity gates (3.5) and the `ci-pr.yml` vs `ci-nightly-build.yml` build/test matrix.

---

## 6. Dangling references found while checking (stale comments or docs, not code)

- `scripts/dev-local.sh:13` -> `scripts/package-local.sh` (does not exist).
- `scripts/check-ui-primitives.mjs:41` comment -> `scripts/ui-primitives-baseline.json` (gone; comment says it moved).
- `scripts/ci-classify-changes.test.mjs` -> `tools/muxlog/index.mjs` (does not exist; fixture path string).
- `scripts/check-comment-hygiene.test.mjs` -> `scripts/gone.js`, `scripts/query.js` (fixture strings).
- `tools/tests/bench-aggregate.mjs:29` and `tools/tests/baselines/README.md` -> `agent.thinkpad-x250.json`, `term.thinkpad-x250.json` (baselines dir holds only a README; files are generated by `--update-baseline`).
- `scripts/cef-build/*.gn` and `configure-cef-build.sh` -> `tools/gn_args.py`, `tools/translator.py` (Chromium/CEF checkout paths, not this repo).
- `.githooks/pre-commit` -> `npm run setup-hooks` (nonexistent).
- `CONTRIBUTING.md:52-55` -> `task storybook` (4.1). `scripts/package-agent.cmd` header -> removed CLAUDE.md (1.9).

---

## 7. Items checked with no problem found

- `docker/` (2 files): `Dockerfile.agent-agentmux` used by `container-image.yml:85` and `pin-consistency.test.ts`; `Dockerfile.srv` used by `srv-image.yml:92`.
- `build/entitlements.mac.plist`: used by `scripts/package-macos.sh` (and its test). The directory contains nothing else.
- `packaging/windows/agentmux.iss` (used by `package-installer.ps1`, Taskfile), `packaging/msix/AppxManifest.xml.template` (used by `package-msix.ps1`), 6 committed PNGs (87 KB) produced by `packaging/msix/generate-assets.ps1` (manual regen tool, documented in its header; keep).
- `.config/nextest.toml`: picked up by nextest automatically (`ci-pr.yml` installs and runs nextest).
- `.githooks/commit-msg`, `pre-commit`: enabled by `package.json` `prepare` (`core.hooksPath .githooks`), not by name references.
- `.github/lychee.toml` used by `weekly-web-links.yml`; `.github/ISSUE_TEMPLATE/*` are GitHub-automatic.
- `scripts/check-file-sizes.baseline` and `scripts/check-time-helpers.allow`: every listed file exists (checked); the ratchets are current.
- `scripts/test-fixtures/gen-docs-index/*`: consumed by `gen-docs-index.test.mjs`, `check-file-sizes.test.mjs`, `ci-classify-changes.test.mjs`.
- All `scripts/check-*.sh|mjs`, `ci-*.mjs`, `gen-*.mjs`, `cef-build/*`, `perf/memory-lab/*` (except via the workflows noted) are invoked by a workflow, Taskfile, vite config, or another live script.

---

## 8. Totals (size of material if the items are accepted)

| Group | Files | Lines | Confidence |
|---|---|---|---|
| verify-package.sh | 1 | 163 | CONFIRMED |
| benchmarks/measure-performance.{ps1,sh} | 2 | 443 | CONFIRMED |
| scripts/dev-tools | 10 | 381 | LIKELY |
| tools/sandbox | 12 | 2427 | LIKELY |
| enable-crash-dumps.reg | 1 | 25 | LIKELY |
| import-agents.sh | 1 | 530 | LIKELY (decision) |
| wipe-old-data-dirs.sh, migrate-branch-to-crates.sh | 2 | 360 | LIKELY |
| package-agent.cmd, test-app-api.mjs, test-splash.sh | 3 | 216 | LIKELY |
| Taskfile artifacts:*, vars, copyfiles, storybook x2 | - | about 85 | CONFIRMED |
| release.yml commented Phase 4 | - | 88 | LIKELY (parked on purpose) |
| bump-wrapper.sh portable block | - | about 55 | LIKELY |
| input-handler-guardrails `layout-reads` job | - | about 14 | CONFIRMED |
| Tests not run by anything (fix wiring, not delete) | 7 | 689 | CONFIRMED |
| Doc-only investigation tools (ui-screenshots x5, macos-paint x13) | 18 | about 1087 | LIKELY (human call) |

Not checked / could not verify: external consumers (private release monitor, cloud repo workflows, `repository_dispatch` senders, agentmux-landing); whether the unwired `.test.sh` files pass if wired in; actual GitHub run history (is `memory-soak` still run by hand; do schedule triggers still fire); docs link rot beyond what appeared in script-path checks; `scripts/ui-screenshots` internals (only checked that README documents the pipeline and helper modules are reachable from `capture.mjs` / `refresh.mjs`).
