# ANALYSIS: dead code, dead scripts and tree shaking in AgentMux, 2026-10-20

**Date:** 2026-10-20
**Status:** analysis. Static findings from a read-only audit of main at v0.59.35, not confirmed by a compile; none of them is implemented. Re-verify each item with grep and the compiler at the start of its PR.

**Author:** Lazo@Area54 (analysis run by a read-only subagent; I have not re-verified every item, see §9)
**Asked by the operator:** "check our code/scripts/dead code/tree shaking"

**What was audited**
- `origin/main` at `f755cd01c` (v0.59.35, 2026-10-20), extracted with `git archive` to `scratch/src`. No repo file, branch, commit or PR was touched.
- Bundle numbers come from the existing v0.59.33 build in `agentmux-fe/dist/frontend`. The portable folder on the Desktop has the same chunks (72 MB).
- No cargo command and no production build was run.
- Frontend tools, run outside the repo against the snapshot: `knip` 5.88.1 (CI mode and production-only mode), source-map byte attribution per package, and small Node scripts for generated types, RPC stubs and SCSS classes.
- Per-item evidence: `scratch/partial_rust.md` (Rust) and `scratch/partial_scripts.md` (scripts, Taskfile, workflows).

**Confidence labels:** CONFIRMED means zero references after the greps listed. LIKELY means a human check is needed.

## 1. Summary

The repo is in good shape. `cargo machete` and `knip --dependencies --files` are CI gates and they hold. What is left is dead code hidden behind `#[allow(dead_code)]`, Wave/Tauri/Electron-era scripts and tasks, small config leftovers, and a bundle shape that needs owner decisions.

| Area | CONFIRMED | LIKELY | Notes |
|---|---|---|---|
| Rust | 12 | 8 | About 97 zero-reference items (~570 lines), 81 test-only items (~1,390 lines). 365 `#[allow]`s, 229 of them `dead_code`, 13 crate-wide. 0 unused deps. |
| Frontend / TS | 8 | 9 | 62 unused exports, 956 unused type exports, 0 unused production files. 47 RPC stubs have no frontend consumer. |
| Dependencies | 2 | 5 | 1 unused devDependency, 1 stale `workspaces` entry. |
| Bundle | 3 | 6 | 42 MB of source maps shipped, 3.4 MB of Shiki grammars, one 4.5 MB main chunk. |
| Scripts / workflows | 9 | 12 | 7 test files that nothing runs. |
| Recent-removal leftovers | 4 | 5 | Comments and small vestiges only. |

Realistic removable volume if everything lands:
- About 2.0 to 2.4k production Rust lines and about 1k Rust test lines.
- About 4.3k lines of scripts and tools.
- 16.8 MB of unreferenced PNGs.
- Source maps (about 28 MB), Shiki grammars (up to 3.4 MB) and PDF fallbacks (about 0.6 MB) are decisions about what to ship, not dead code.
- Rust binary size is about unchanged, because the linker already strips dead code.

**False positives to know about**
- A local `knip` run flags 7 unused devDependencies and `test/jsdom-ts-tests.ts`. Six of those dependencies are imported by `vite.config.ts` and the eslint configs, and the file is imported by `vitest.config.ts`. This is because the snapshot's `node_modules` is a junction. Only `semver` is real (D-1).
- The scripts audit first said nothing invokes `knip`. That is wrong: `ci-pr.yml:834` runs `npx knip --dependencies --files --no-progress`. `knip.json` is live and must stay.

### Top 10 wins, ranked by value over risk

| # | Win | Size | Risk |
|---|---|---|---|
| 1 | Delete broken Taskfile and package.json entries: `storybook`, `storybook:build`, the `package:portable` script, the `artifacts:*` tasks and their vars, `copyfiles:*:*` | ~85 lines | none |
| 2 | Delete the dead ijson stack in srv (`backend/ijson.rs`, `filestore/ijson.rs`, 2 option fields, tests) | ~590 prod + ~400 test lines | low |
| 3 | Delete `srv/src/backend/userinput.rs` and the never-emitted userinput modal chain in the frontend | 383 Rust + ~240 TS/SCSS lines | low |
| 4 | Delete scripts for stacks that no longer exist: `verify-package.sh`, `benchmarks/measure-performance.*`, `scripts/dev-tools/`, `tools/enable-crash-dumps.reg`. Add `tools/sandbox/` (2,427 lines) after owner OK. | ~1,000 lines, plus the sandbox | none to low |
| 5 | Config cruft: the `streamdown` `@source` line in Tailwind, the tsconfig `@/shadcn/*` alias and `experimentalDecorators`, `workspaces: ["docs"]` with no `docs/package.json`, the `semver` devDependency, the never-run `eslint.config.js` | a few lines each | none |
| 6 | Delete three unreferenced box-art PNGs in `assets/` | 16.8 MB | none |
| 7 | Wire up the 7 tests nothing runs, or delete them | 689 test lines | low |
| 8 | Delete Go-port leftovers: 36 unused structs and consts in `rpc_types/` and 56 unreferenced declarations in `frontend/types/srv-types.d.ts` | ~190 Rust lines + types | low |
| 9 | After wins 2, 3 and 8, remove the 13 crate-wide `#![allow(dead_code)]`s and delete the allow-hidden dead items | ~570 lines | low, needs one compile |
| 10 | Bundle, owner decisions: drop `sourcesContent` from shipped maps (about 28 of 42 MB), restrict Shiki to needed grammars (about 3.4 MB), lazy-load heavy panes | see §5 | low to medium |

## 2. Rust

Static analysis only. Every item was re-grepped with `rg -w` over the repo (excluding `node_modules` and `docs`). srv, launcher, mcp and bashwrap are bin-only crates, so rustc already warns on unused items. Surviving dead code is almost all under `#[allow(dead_code)]`.

- **R-1 ijson stack (CONFIRMED).**
  - Files: `crates/srv/src/backend/ijson.rs` (791 lines), `storage/filestore/ijson.rs` (125), mod lines `backend/mod.rs:66` and `filestore/mod.rs:18`, and the `FileOpts.ijson` / `ijsonbudget` fields (`filestore/types.rs:19,21`, `rpc_types/misc.rs:111,113`).
  - Evidence: only `filestore/tests.rs:599,620` call `append_ijson`; `rg 'append_ijson|compact_ijson' crates` shows no production caller.
  - Action: delete the files, mod lines, fields and tests (`filestore/tests.rs:584-660`). Regenerate the ts-rs types.
  - Risk: low to medium (ts-rs regeneration). Saves ~590 prod + ~400 test lines.
- **R-2 `srv/src/backend/userinput.rs` (CONFIRMED).**
  - 383 lines (203 prod), `pub mod userinput;` at `backend/mod.rs:149`.
  - Outside the file, `rg` finds only a stub arm `("userinput","SendUserInputResponse")` at `server/service/misc.rs:16` whose body is "Accept but drop", plus comments. `EVENT_USER_INPUT` (`mps.rs:58`) is never emitted.
  - Action: delete the module. Pair with F-1.
  - Risk: low.
- **R-3 Go-port wire types in `srv/.../rpc_types/` (CONFIRMED unreferenced in Rust).**
  - 36 items, ~190 lines, under the blanket `#![allow(dead_code)]` of `rpc_types/mod.rs`.
  - They are consts in `commands.rs:9-29` and `:775-776`, structs in `misc.rs`, `block.rs:29,37,59,65` and `agent.rs:73`, and `mod.rs:112 is_rpc_request`.
  - Caveat: `CommandMessageData` is used at `frontend/app/store/rpc-client.ts:151`. `CommandDeleteBlockData` has a generated TS file and a re-export at `rpc-api/block.ts:43`. Check those two before removing.
  - Risk: low except those two.
- **R-4 Allow-hidden dead items (CONFIRMED).**
  - About 97 items, ~570 lines. Full list with line numbers is in `partial_rust.md` section 2a.
  - Largest groups:
    - `wcore/event.rs:13` (the whole 46-line file).
    - `base.rs` and `shellexec.rs`: about 25 consts and fns.
    - `blockcontroller/mod.rs`: 7 `META_KEY_*` consts and `stop_all_controllers`.
    - `mps.rs`, `messaging/`, and a dozen small items in `eventbus.rs`, `lan_discovery.rs` and `reactive/*`.
    - `launcher/src/host_pipe/connection.rs`: a 46-line file that is one unused struct.
    - `cef/src/memory_pressure.rs:153`.
  - Action: delete. The compiler catches any miss.
  - Risk: low.
- **R-5 Test-only items and blanket allows.**
  - About 81 items (~1,390 lines) are used only by tests. Biggest:
    - `service.rs get_method_meta` (261 lines).
    - `reactive/sanitize.rs wrap_jekt_message` (143).
    - `reactive/handler.rs try_register_agent_with_nonce` (77).
    - `shellexec.rs MockConn`, a test double in production code.
  - Leave the 12 `app_server_protocol` methods (305 lines). They are annotated "Unwired until SPEC_CODEX_APP_SERVER_FIRST_CLASS_PROVIDER Slice 6".
  - The 13 crate-wide `#![allow(dead_code)]` files are listed in `partial_rust.md` 2c.
  - `srv/src/registry/mod.rs:18` looks removable (LIKELY).
  - 63 srv and 13 cef allows may be stale, but only a compile settles that.
  - Risk: low, but needs one `cargo check`. Warn before triggering it, since rebuilds are slow (the debug cache was cleared on 2026-10-18).
- **R-6 Duplicated helpers (CONFIRMED identical bodies, ~150 lines, low value).**
  - `fnv1a_64` in common and launcher.
  - `valid_session_id` in remote and srv.
  - `suppress_os_crash_dialogs` in cef and launcher.
  - `version_core` in launcher and srv.
  - Also `extract_commented_setting_key`, `bind_first_pipe_instance`, `merge_known`, `marker_version`.
  - `UIContext` is defined twice in srv (`obj.rs:189`, `service.rs:37`).
- **R-7 Cargo hygiene.**
  - `srv/Cargo.toml` dev-dependencies `serde_json` and `libc` duplicate the normal deps (CONFIRMED).
  - The `launcher/Cargo.toml:41-45` comment still cites "the vacuum cutoff parameter" as the reason for chrono `serde` (LIKELY).
  - Duplicate versions: `axum` 0.7 vs 0.8, `which` 7 vs 6, `png` 0.17 vs 0.18, `thiserror` 1.x vs 2.x, `dirs` 5 vs 6. Unifying axum is real migration work (risk medium).

**Checked and clean:**
- No unused dependency in any of the 8 manifests.
- All 4 features are enabled somewhere.
- All 8 crates and all binaries are built or packaged.
- No orphan `.rs` files (1,283 checked).
- All `include_str!` assets are used.
- No commented-out code blocks.

## 3. Frontend / TypeScript

- **F-1 Never-emitted userinput modal chain (LIKELY).**
  - Files: `frontend/app/modals/userinputmodal.tsx` (180 lines) and `.scss` (57), `store/services.ts:117-124`, the `WpsEvent.UserInput` subscription at `store/global.ts:307-313`, and `UserInputRequest` in `srv-types.d.ts`.
  - Reason: srv never emits the event (R-2), so the modal cannot open.
  - Caveat: `WpsEvent.UserInput` comes from `gen-mux-events`, so remove it from the Rust event list too. Check that nothing outside srv emits it.
  - Action: delete together with R-2.
  - Risk: low to medium.
- **F-2 Dead component `frontend/app/view/bundle-summary.tsx` (132 lines) plus `bundle-summary.scss` (CONFIRMED).**
  - Production-mode knip lists it as unreachable. Only its own test imports it.
  - Two comments (`identity-manager.tsx:10`, `identity-pane-view.tsx:7`) say the panel was removed.
- **F-3 Dead global atoms (CONFIRMED).**
  - In `store/global.ts` and `types/custom.d.ts`:
    - `typeAheadModalAtom` (signal never set).
    - `reinitVersion` (set at `app-init.ts:776`, never read).
    - `hasCustomAIPresetsAtom` (`config-signals.ts:21-30`, Wave-era).
  - `schema/aipresets.json` has no reference either.
  - The bag entries `flashErrors`, `lanInstancesAtom` and `clientId` are also never read through `atoms`. Consumers import them directly.
  - Keep `uiContext`, which is read dynamically at `mos.ts:119`.
- **F-4 RPC stubs with no frontend consumer (LIKELY, generated and intentional).**
  - `rpc-api/generated.ts` has 324 stubs. 47 have no reference in `frontend/`, `test/`, `scripts/` or `sdk/`, and 2 are test-only.
  - They include `Mcp*`, `Skill*`, `FleetGroup*`, `EventSub*`, `Route(Un)announce`, `WslList`, `ConnList`, `MuxInfo`, `NotifyAck`, `WidgetApi`, `GetMeta` and the agent-history, skill and instance CRUD stubs.
  - The srv handlers are real and used by MCP, the CLI or other clients, and event and route commands are wire names, so only the TS stub is unused.
  - Removing the non-wire ones saves about 3 to 4 KB and ~400 source lines. Edit `generated-manifest.json` and rerun `scripts/gen-rpc-stubs.mjs`.
  - Risk: low.
- **F-5 Unused types (no bundle effect).**
  - Of 747 generated `frontend/types/rpc/*.ts` files, 56 are unreachable from any non-generated file. These are app-API request and response types, and they are harmless. `rpc-schema.json` (184 KB) is consumed by the generator and CI; keep it.
  - `srv-types.d.ts` (1,511 lines) is a Go-era mirror. 56 of its 114 top-level declarations are unreferenced outside the file, such as `AIAttachedFile`, `Packet`, `TEvent*`, `UIChat`, `WebBookmark`, `FileOpts`, `CommandAppendIJsonData` (CONFIRMED).
  - knip reports 956 unused exported types, 485 of them in `rpc-api/*.ts` as historical-name re-exports.
- **F-6 Unused and test-only exports (low).**
  - 62 unused exports, for example `timeBlock`, `timeAsync`, `perfStore`, `setObjectValue`, `subscribeSrvEvent`, `notify`, `columnsFor`, `rowEnd`, `AccountsTab`, `LoadCell`.
  - Two duplicate exports: `live-feed.ts` and `platformutil.ts`.
  - 689 more exports are used only by tests. These are mostly intentional seams.
  - `sdk/widget-sdk` shows as unreachable but is served by srv; keep it.
- **F-7 Unused CSS.**
  - A heuristic found 213 candidates, nearly all false positives from library classes and template-built names.
  - Only `.agent-create-tool-link-arrow` (`AgentCreatePanel.scss:69`) is plausible. `.ui-density-comfortable` and `:root.am-uniform-fade` (`theme.scss:611,641`) need a glance.
  - Not worth its own PR.
- **F-8 Config leftovers (CONFIRMED).**
  - `frontend/tailwindsetup.css:5` has `@source "../node_modules/streamdown/dist/index.js"`. `streamdown` is not a dependency.
  - `tsconfig.json` has the path `@/shadcn/*` (the directory does not exist) and `experimentalDecorators` (no decorators in `frontend/`; LIKELY safe to remove).

## 4. Dependencies

- **D-1 `semver` devDependency (`package.json:64`).** No import or command use anywhere (the word appears in prose and in Docker `type=semver`).
  - Why CI's knip does not flag it is unclear. Either `main` is red on that step or something not visible keeps it alive. Check before deleting.
- **D-2 `"workspaces": ["docs"]`.** `docs/package.json` does not exist, there is no `node_modules/docs` link, and no script uses `--workspace`. LIKELY stale. Do it with D-1 so the lockfile is touched once.
- **D-3 Dev dependencies with narrow use.**
  - `eslint.config.js` is the general config and is wired to no script, workflow or hook. Its own comment says it has never run successfully.
  - Only `eslint.theme-colors.config.js` runs, in CI. `@eslint/js` and `eslint-config-prettier` serve only the general config.
  - Decide: delete them, or wire a `lint` script.
  - `postcss` has no import and no config (likely droppable).
  - `tsx` is used only by the manual bench `tools/tests/bench-markdown-parse.mjs`.
- **D-4 Single-use runtime dependencies (low value).**
  - `sprintf-js` (one call at `ws.ts:141`; it also has an open npm advisory with no fix, see the security triage report).
  - `colord` (one call at `termutil.ts:50`).
  - `rxjs` (only `Subject`, plus a stray type import at `custom.d.ts:12`).
  - `css-tree` (189 KB in the main chunk) is used once, in `muxutil.ts`, to vet CSS background URLs. Replacing it is security-adjacent, so risk is medium.
- **D-5 Overlapping libraries.** Three highlighters ship: `highlight.js` via `rehype-highlight` (167 KB in main), Shiki, and CodeMirror language packs. The markdown fences could use the lazy Shiki path instead. That is an owner decision.

**Checked and clean:** every production dependency has an import, or is used for its SCSS theme (`highlight.js`). The `@emnapi/*` pins are intentional.

## 5. Bundle / tree shaking

**Output shape**

| Item | Size |
|---|---|
| `dist/frontend` total | 72 MB |
| JS (359 files) | 14.1 MB |
| Source maps (356 files) | 42.1 MB (58%) |
| CSS (one file) | 0.80 MB |
| wasm (4) | 0.92 MB |
| pdf.js `.bcmap` (34) | 1.08 MB |
| `licenses/` | 9.4 MB, of which `CHROMIUM_CREDITS.html` is 8.8 MB |

Largest JS chunks: `index` 4.51 MB, `pdf.worker.min.mjs` 1.26 MB, mermaid `cynefin` 689 KB, Shiki `cpp` 626 KB, `mermaid.core` 624 KB, Shiki `wasm` 622 KB, `pdfjs-runtime` 498 KB, `openjpeg_nowasm_fallback` 446 KB, `cytoscape` 444 KB, `katex` 261 KB.

**What is in the 4.5 MB main chunk**
- App code: ~2.5 MB, of which `app/view` is 1.5 MB.
- `@xterm/*`: ~510 KB.
- CodeMirror: ~400 KB.
- `css-tree`: 189 KB.
- `highlight.js`: 167 KB.
- `parse5`: 124 KB.
- `uplot`: 54 KB.
- `source-map-js`: 29 KB.

**Findings**
- **B-1 Source maps ship in full (owner decision).**
  - `vite.config.ts:163-170` sets `sourcemap: true` on purpose, for the runtime stack resolver.
  - In the main map, `sourcesContent` is 14.5 of 20.8 MB. The resolver uses only `originalPositionFor`.
  - `build.rollupOptions.output.sourcemapExcludeSources: true` would drop an estimated 28 MB. The cost is that DevTools no longer shows original source in a packaged build.
  - Risk: low.
- **B-2 Shiki grammars.**
  - 73 chunks, ~3.4 MB of language grammars plus the 0.6 MB oniguruma wasm.
  - `shiki-highlighter.ts` registers all of `webLanguages` plus 24 extras. They load on demand, so there is no startup cost, only disk and installer size.
  - An explicit grammar list would cut up to ~3 MB of JS and its maps.
  - Risk: low. Missing languages fall back to plain text.
- **B-3 No route-level code splitting (LIKELY).**
  - `block-registry.ts` statically imports 19 pane manifests.
  - The only dynamic imports are mermaid, the PDF runtime, Shiki and the CodeMirror language packs.
  - The editor (~400 KB), sysinfo (`uplot`), vault, tower, swarm and warden all sit in the startup chunk.
  - `manualChunks` is disabled with a comment citing "the old WebKitGTK host". The same file says CEF has been the only host since, so that reason is gone.
  - Measure parse time before and after. Risk: medium, because of keep-alive panes, HMR and startup ordering.
- **B-4 Markdown renderer weight (LIKELY).** `markdown.tsx` statically pulls `highlight.js` (167 KB) and `parse5` (124 KB). Mermaid, KaTeX and the PDF viewer are already lazy.
- **B-5 Mermaid, ~3 MB across 38 lazy chunks.** This includes diagram types the app likely never needs (cynefin, swimlanes, venn). No startup cost. Pruning needs a Vite plugin that aliases the dynamic imports; only worth it if install size matters.
- **B-6 PDF fallbacks.** `openjpeg_nowasm_fallback` (446 KB) and `jbig2_nowasm_fallback` (152 KB) are for environments without WebAssembly, which CEF always has (~0.6 MB).
- **B-7 Tree shaking is healthy.**
  - `import * as` is used only for internal namespaces. `widget-loader.ts` re-exposes solid to widgets on purpose.
  - No CommonJS `require()` in `frontend/`.
  - Only small `index.ts` barrels (14) and three `*.platform.ts` shims.
  - Only three side-effect imports: `block-registry` and two diag modules.
  - No lodash, moment or icon pack is imported whole.
  - `package.json` has no `sideEffects` field. That is fine for a private app, and adding one is low value.

## 6. Scripts and workflows

Full detail is in `partial_scripts.md`.

- **S-1 Broken entries (CONFIRMED).**
  - `Taskfile.yml:98-110` `storybook` and `storybook:build` run npm scripts that do not exist. `CONTRIBUTING.md:52-55` still documents `task storybook`.
  - The `package.json` script `package:portable` calls a task that does not exist.
  - `dev:cleardata:windows` (~line 445) uses `%LOCALAPPDATA%` where the shell will not expand it (LIKELY a silent no-op).
- **S-2 Dead legacy tasks (CONFIRMED).**
  - `artifacts:*` tasks (lines 370-426, ~57 lines) and the vars `ARTIFACTS_BUCKET`, `RELEASES_BUCKET`, `WINGET_PACKAGE` (lines 20-26).
  - `copyfiles:*:*` (line 471) and `build:frontend:dev` (line 302) have zero references.
  - `run` (lines 1383-1400) hard-codes port 5173 (LIKELY stale).
- **S-3 Retired-stack scripts (CONFIRMED).** `scripts/verify-package.sh` (163 lines, Electron/Wave) and `scripts/benchmarks/measure-performance.{ps1,sh}` (443 lines, src-tauri/WaveMux). `agent-open.mjs` in the same folder is live; keep it.
- **S-4 `scripts/dev-tools/*.ps1`** (10 files, 381 lines) (LIKELY dead). SendKeys and screen-scrape hacks, superseded by CDP and the MuxAPI `UI` tool.
- **S-5 `tools/sandbox/`** (12 files, 2,427 lines) (LIKELY dead). WaveMux-era Parsec host provisioning with about 140 `wavemux` mentions, referenced by nothing outside itself.
- **S-6 One-offs and superseded scripts.**
  - `tools/enable-crash-dumps.reg` matches the literal `agentmux-srv.exe`, but the shipped exe is versioned. `register-crash-dumps.ps1` replaced it.
  - `wipe-old-data-dirs.sh` (236) and `migrate-branch-to-crates.sh` (124).
  - `import-agents.sh` (530) is mostly superseded by `def_migrate.rs`, but it holds the only rehydrate logic. It needs a decision.
  - `package-agent.cmd` (19) cites a CLAUDE.md that is gone.
  - `test-app-api.mjs` (125) and `test-splash.sh` (72) have no runner.
  - `bump-wrapper.sh`: the ~55-line portable-bump branch (lines ~36-52 and 112-163) is dead. The wrapper itself is live.
- **S-7 Unreferenced assets (CONFIRMED).** `assets/agentmux-box-art-brain-alternate*.png` (three files, 16.8 MB). Only one 2026-05 retro doc mentions them. Git history keeps them, so deleting only shrinks working trees.
- **S-8 Tests nothing runs (CONFIRMED).**
  - The 53 `scripts/**/*.test.mjs` do run, via the vitest default include.
  - These do not:
    - `linux-apprun.test.sh`
    - `stage-linux-runtime.test.sh`
    - `package-macos.test.sh`
    - `verify-angle-libs.test.sh`
    - `tools/tests/lib/{bench-stats,instance-discovery}.test.mjs` (vitest excludes `**/tools/**`)
    - `tools/sandbox/tests/Sandbox.Tests.ps1`
  - Whether they pass if wired in was not checked.
- **S-9 Workflows.**
  - There is no `if: false`, no uncalled `workflow_call` workflow, no unused action pin, and no dangling script path.
  - `ci-pr.yml` outputs `docs_only` and `version_only` (lines 137 and 139) are never read (CONFIRMED).
  - `srv-image.yml:22` filter `'*/Cargo.toml'` matches nothing (CONFIRMED).
  - `input-handler-guardrails.yml` job `layout-reads` is triplicated in `ci-pr.yml:705`, `ci-nightly-build.yml:323` and Taskfile. Its header is stale. `sync-ipc` runs only here.
  - `memory-soak.yml` is named "nightly" but has no schedule (LIKELY: rename).
  - `build-*.yml` declare `repository_dispatch` with no in-repo sender (LIKELY external).
- **S-10 Duplicates.**
  - The two `fake-anthropic.mjs` files (104 and 155 lines) are mergeable with effort.
  - `gen-docs-index.sh` and `.mjs` are both pinned by CI; leave them.
  - `dev:serve` and `dev:standalone:serve` share Vite-start logic.
- **S-11 Doc-only investigation tools (human call).** `scripts/ui-screenshots/*` (5 files, 586 lines), `scripts/perf/macos-paint/*` (13 files, 501 lines), `tab-switch-report.mjs`, `docs-status-origin-survey.mjs`, and the `verify-shortcuts*` family. Most are documented manual tools. The experiment-named ones look finished.
- **S-12 Parked code.** `release.yml:426-513` has 88 lines of winget and ms-store jobs commented out on purpose on 2026-08-12. Keep until the winget decision.

## 7. Leftovers from recent removals

| Removal | Finding | Confidence |
|---|---|---|
| Vacuum row / `run_saga_vacuum` | **Clean.** No RPC stub, event, config key, frontend string, generated binding or test remains. The terms appear only in `docs/specs/SPEC_UPGRADE_PANEL_2026_06_27.md`. `MaintenanceSection.tsx` no longer has a Vacuum row. | CONFIRMED |
| Saga log made in-memory | Stale `LSD-1/2/3` comments. `launcher/src/saga/mod.rs:87-88` says the opposite of the code beneath it, and about 25 more lines in `saga/mod.rs`, `supervisor/{unix.rs:37,798; windows.rs:372,1297}` and `main.rs:369` describe the durable log. | CONFIRMED stale |
| Saga log | `diag.rs:694-710` documents a SQLite reader that no longer exists. `run_sagas_diag` has two identical cfg wrappers (`:711`, `:716`), and `sagas_diag_tests` (`:750+`) tests only the in-memory log. | CONFIRMED stale |
| Saga log | `SagaOutcome::FailedCompensation` and `mark_failed_compensation` have no production caller (`saga/log/mod.rs:49,83-84,367`, `tests.rs:113-153`). About 30 lines including 4 tests. | CONFIRMED test-only |
| Saga log | `StepRecord` / `SagaRecord` fields are kept only for tests (`saga/log/mod.rs:155-200`). The chrono `serde` comment at `launcher/Cargo.toml:41-45` is stale (R-7). | LIKELY |
| Saga log | Best-effort deletion of legacy `launcher-sagas.db` (`supervisor/windows.rs:331-339`, `unix.rs:258-262`). | Intentional; drop after a few releases |
| tar.exe update path | No code invokes `tar.exe`; Windows unpacks with `sevenz-rust2`. `tar_exe()` (`update_download.rs:506-509`) is a trivial wrapper that returns `"tar"`, used at `:549` and in a test at `:799`, and named in the audit table at `pane_env.rs:443`. | CONFIRMED vestigial |
| tar.exe | The non-Windows branch still imports `NoWindow` and calls `.no_window()` (`update_download.rs:546-551`). The `:443` doc is half true now. The `tar` crate dep is still used. | LIKELY tidy-up |
| Frontend | The `saga-event-observed` event type (`launcher-event/reducer.ts:71`, `types.ts:109`, `launcher-event-store.ts:167`) only logs raw saga events. Probably still live. | Verify |

## 8. Proposed PR breakdown

Each PR needs a `.changesets/*.md` from `scripts/changeset.sh <patch|minor|major> "<desc>"`. Never edit version files. PR titles start with `<agent>@<host>: `.

| PR | Contents | Changeset | Verify with |
|---|---|---|---|
| 1. Taskfile / package.json cleanup | S-1, S-2, the `CONTRIBUTING.md` storybook paragraph | patch | `task --list`, docs gates |
| 2. Stale scripts and tools | S-3, S-4, S-5 (after owner OK), S-6 (except `import-agents.sh`), `bump-wrapper.sh` block, dangling comments | patch | CI `docs` job |
| 3. Unreferenced assets | S-7 | patch | file-size and link checks |
| 4. Unrun tests | S-8: add the `.test.sh` files and a `node --test` step, or delete | none (CI only) | run them once locally first |
| 5. Workflow tidy | S-9: unread outputs, `*/Cargo.toml` filter, `layout-reads` job, rename `memory-soak` | none | workflow lint |
| 6. Frontend config leftovers | F-8, D-1, D-2, D-3 | patch | vitest, tsc, `npm ci`, knip |
| 7. Dead frontend code | F-2, F-3, one F-7 selector | patch | vitest, tsc, knip |
| 8. Rust ijson stack | R-1 plus regenerated rpc types | patch | `cargo check --tests` in CI |
| 9. userinput, Rust and frontend | R-2 + F-1 | patch | cargo check, tsc, `check-rpc-bindings.sh` |
| 10. Go-port leftovers | R-3 + F-5 | patch | cargo check, tsc |
| 11. Allow-hidden dead code | R-4, R-5, then remove the 13 crate-wide allows | patch | `cargo check --tests` (after PRs 8 to 10) |
| 12. Saga and tar leftovers | §7, R-6, R-7 | patch | `cargo check --tests` |
| 13. RPC stub trimming | F-4 (non-wire stubs only) | patch | gen-rpc-stubs test, tsc |
| 14. Dependency swaps (optional) | D-4: `sprintf-js`, `colord`, `rxjs` | patch | vitest |
| 15. Bundle changes (owner decisions) | Separate PRs for B-1, B-2, B-3/B-4, B-6 | patch or minor | production build, startup timing, smoke of each pane |

PRs 1 to 7 are independent. Land 8, 9 and 10 before 11. Measure `dist/frontend/assets` sizes before and after each PR 15 item.

## 9. What could not be checked

- **Rust compile-dependent claims:**
  - Nothing was compiled. Stale-allow claims (63 srv, 13 cef), `pub use` unused imports and test-only items are unverified.
  - Per-platform deadness is not separated.
  - Enum variants and fields under allows were listed, not judged.
- **RPC command names registered by string in srv** were not cross-checked against frontend, MCP and CLI callers.
- **External consumers:** the private release monitor, cloud repos, `repository_dispatch` senders, `agentmux-landing`, and GitHub run history.
- **Whether the unwired `.test.sh` files pass** if wired in.
- **Bundle numbers** come from the v0.59.33 build, not v0.59.35, and B-1 to B-4 savings are estimates from source-map attribution, not a measured build.
- **Why CI's knip step does not flag `semver`** (D-1).
- **I have not independently re-run the subagent's greps.** Treat CONFIRMED as "the agent found zero references", and re-check each item at the start of its own PR.

Agent: Lazo@Area54
