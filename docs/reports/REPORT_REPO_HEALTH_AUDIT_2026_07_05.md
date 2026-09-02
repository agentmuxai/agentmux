# AgentMux Repo Health Audit — Code, Architecture, Tree-Shake, Docs, Cross-Repo

**Date:** 2026-07-05
**Baseline:** `agentmux` main @ `183aecc4` (v0.50.0) · `agentmux-cloud` origin/main @ `89e59a4` · `agentmux-docs` main @ latest (site v0.1.6, prod deploy pinned at 0.1.4 / 2026-06-23)
**Method:** six parallel deep-analysis passes (architecture, Rust dead code, frontend dead code, code hygiene, docs audit, cross-repo triangulation), each verified against source with file:line evidence. Read-only — no changes made.
**Excluded throughout:** the 11 modified + 2 untracked working-tree files from the in-flight floating-pane ghost-landing fix.

---

## 0. Executive summary

The codebase is structurally healthier than its size and age suggest: compile-time layering is exemplary (strict star topology through `agentmux-common`), doc culture is unusually strong (913 docs, near-universal Status/Date headers), test discipline is good where it exists, and there are **zero** FIXMEs, commented-out code blocks, or orphaned `.rs` files.

The dominant debt is not tangles — it is **three large migrations all frozen one step before their risky cutover**, each paying a double-path tax in production:

1. **Layout single-writer authority (#864 / Phase E / Strong Reducer Authority)** — the entire engine is built and dormant (all 11 reducer arms, `balance_node`, persist arms) while production still runs the legacy full-tree `UpdateObject` path plus 7 wcore-direct writers, with no CAS and a `heal_layout` backstop papering over the split-brain.
2. **Quit authority (Pillar 2)** — `reconcile_quit` is wired but nothing consumes it; the 3–4-way quit split-brain (client callback, WRR, orphan_reconcile, legacy gate) is still live.
3. **Agents dual-write (Phase 3c)** — readers flipped to `db_agents`, but every mutation still mirrors into legacy tables through ~1,600 lines of transitional scaffolding whose own header says "This whole file goes away in Phase 3c."

Beyond the migrations, the concrete cleanup surface is large and mostly low-risk:

- **~1,000 LOC of high-confidence dead Rust** deletable today (~2,300–3,300 ceiling), plus 2 unused Cargo deps
- **~4,200 LOC of verified-dead frontend files**, 3 removable runtime npm deps, 26 phantom (unlisted) deps
- **~2,000 LOC of near-pure duplication** in platform-variant triplets (`TileLayout.*` ~95–97% identical, `zoom.*` ~100% identical)
- **Three top-level docs (BUILD.md worst) with outright-wrong claims** (Tauri-era WebView2, NSIS-vs-Inno, React-vs-SolidJS, 9-vs-17 widgets)
- **One public-facing product contradiction**: the docs site says AgentMux runs no relay ("bring your own open-source muxbus-server") while agentmux-cloud ships a proprietary metered relay billing per jekt

---

## 1. Architecture assessment

### 1.1 Workspace map

Rust crates (all v0.50.0, workspace-inherited):

| Crate | Files / LOC | Responsibility |
|---|---|---|
| agentmux-srv | 272 / 109,251 | Backend sidecar: axum RPC, SQLite, agent runtimes, identity, sagas, reducer |
| agentmux-cef | 81 / 37,573 | Host: bundled CEF 148, window/pane/pool mgmt, IPC bridges, host reducer |
| agentmux-launcher | 48 / 22,848 | Privileged root: Job Object, single-instance pipe, saga coordinator, splash |
| agentmux-common | 9 / 5,570 | Shared leaf: IPC wire protocol (`ipc.rs`, 2,095 LOC), DataPaths, RuntimeMode |
| agentmux-bashwrap | 4 / 2,610 | PTY bash wrapper + PreToolUse hook |
| agentmux-mcp | 1 / 1,791 | MCP stdio server — a single-file crate |

Frontend: ~108,800 LOC TS/TSX; `app/view/` 59k, `app/store/` 25k, `layout/` 6.8k, `types/gotypes.d.ts` 2,388 (hand-maintained).

**Dependency graph is a strict star** — launcher/cef/srv/bashwrap/mcp each depend only on common; zero sibling deps. All cross-process coupling is via shared `Command`/`Event` enums + env-var handoffs.

### 1.2 Clean seams (keep doing this)

- One wire-protocol source of truth (`common/src/ipc.rs:4-7`)
- Paths/runtime-mode unified in common (`data_paths.rs`, `runtime_mode.rs`)
- Frontend↔srv: single WS + RPC + WPS pub/sub; **zero store→view import cycles**
- View registration centralized (`block-registry.ts:27-43`)

### 1.3 Leaky seams

1. **Fused two-domain protocol enum** — `ipc.rs` mixes launcher-domain and srv-domain variants in one ~770-line `Command`; each reducer exhaustively no-ops the other domain's variants (`agentmux-srv/src/reducer.rs:22-24,63`).
2. **Host shadow state** — `agentmux-cef/src/launcher_ipc.rs` maintains read-models of launcher-authoritative window state; sanctioned by the 06-30 authority doctrine but only convention keeps it read-only.
3. **View knowledge punctures the generic block frame** — `blockframe.tsx:324-352` (term special case), `:388-414` (browser favicon diag).
4. **Duplicated infrastructure**: three hand-rolled reducers with triplicated `Ctx`/discipline; two saga engines + a receiver (launcher's ~4k-line durability layer self-assessed as "over-engineered … only 2 saga types" in `SPEC_ARCHITECTURE_HEALTH_AND_REFACTOR_2026_06_29.md:48`); `event_log.rs` duplicated verbatim between launcher and srv (srv copy: "lift the shared parts into agentmux-common", `event_log.rs:32-33`); logging setup ×4.

### 1.4 The three frozen migrations (top architectural risk)

**(a) Layout single-writer — engine built, cutover not flipped.**
Done: typed `LayoutNode` (`obj.rs:398-410`), all 11 pure tree helpers **including `balance_node`** (`backend/layout/mod.rs:638`), all 11 reducer arms atomic (`reducer.rs:92-214`), persist arms.
Not done: zero production dispatch sites for `Command::Layout*` tree mutations; the load-bearing `UpdateObject`→`LayoutSetTree` reroute never landed (`server/service/object.rs:226-258` still full-row-writes → **two-writer split-brain on `db_layout` intact**); 7 wcore-direct writers stranded ("E.4 territory": `object_helpers.rs:74`, `layout_helpers.rs:32-181`, `wcore/mod.rs:159-260`, `window.rs:195`, `wcore/dnd.rs`, `wcore/block.rs:83,353`); no CAS (`store.rs:396,418,618` blind `version+1`); frontend 100% on the old path (`layoutPersistence.ts:283-301` debounced full-tree push; keeps its own `balanceNode` — the "second writer" the spec wants retired).
**Stale progress markers mislead:** `reducer.rs:86` still says "4 of 11 arms shipped" (all 11 wired); `SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md` still lists `balance_node` as the missing keystone.
This migration also **blocks Pillar 1** (host disposability), which blocks the saga-layer collapse.

**(b) Quit authority (Pillar 2) — Stage 1 only, behavior-neutral.** `reconcile_quit` wired (`agentmux-cef/src/reducer/quit.rs:56-63`) but nothing consumes `request_drain`; `orphan_reconcile.rs` and `wrr/` still independent deciders. The repo's own audit: ~45–55% of PRs touch memory/lifecycle/crash; renderer-OOM re-fixed 6+ times.

**(c) Agents dual-write Phase 3c — not executed.** `dual_write.rs` (679 LOC, header: "goes away in Phase 3c") + `agents_consolidate.rs` (942 LOC) still hot; every agent mutation mirrors into legacy `db_agent_definitions`/`db_agent_instances`. One PR-sized deletion removes a whole failure mode.

### 1.5 Other structural risks

- **Hand-maintained wire contracts ×2**: `frontend/types/gotypes.d.ts` (2,388 LOC, generator deleted with the Go backend, no drift check vs `obj.rs`/`rpc_types.rs`) and the fused `ipc.rs`. Silent-drift surfaces with no mechanical enforcement.
- **Window-identity resolution multi-path**: four overlapping HWND resolvers (`agentmux-cef/src/commands/window/lifecycle.rs:99,207` + cache `state.rs:763`); ≥8 documented regressions; the P1 fix ("one canonical `window_hwnd(label)`") is partly parked on wip branches.
- **Next-tier god files, unplanned** (the >2.2k tier was successfully modularized — all 4 specs done): `persist_subscriber.rs` (1,966), `cef/lib.rs::run()` (~1,680-line function), `mcp/main.rs` (1,791 single-file crate), `identity/resolver.rs` (1,730); frontend: `app-init.ts` (1,062, 12 unrelated init concerns), `AgentLaunchModal.tsx` (1,045), `tabbar.tsx` (955), `floating-pane-workspace.tsx` (1,056 — also does platform branching inline instead of the `.platform.ts` convention, and has zero tests).

---

## 2. Tree-shake — Rust

`cargo check --workspace`: **120 warnings** (srv 60, cef 51, launcher 4, common 5; mcp/bashwrap 0). Two caveats: the build is Windows-only without `--tests` (many "dead" items are non-Windows impls or test helpers), and **52 files in srv carry `allow(dead_code)`** including whole-file suppressions on `base.rs` (803), `ijson.rs` (789), `userinput.rs` (381), `rpc_types/*` (2,349) — the real dead surface hides behind these.

### 2.1 High-confidence dead (deletable now, ~950–1,100 LOC)

| Item | Location | LOC |
|---|---|---|
| **`userinput.rs` whole module** (Tauri-era, zero refs) | `agentmux-srv/src/backend/userinput.rs` + `backend/mod.rs:36` | 381 |
| Superseded wcore mutation layer (zero-ref subset) | `wcore/window.rs:18,51,139,175`, `tab.rs:15,20,119,135`, `block.rs:14,381`, `workspace.rs:14`, `event.rs` (all) | ~395 |
| Frameless-resize hook cluster | `agentmux-cef/src/client/wndproc.rs:17,38,64-136` | ~114 |
| `build_config_files_with_bus` | `agentmux-srv/src/backend/agent_config.rs:153-239` | ~86 |
| `skill_upsert` + `mcp_server_upsert` (superseded by `_unique` variants) | `storage/skills.rs:329`, `storage/mcp_servers.rs:132` | ~47 |
| cef `AppState` leftovers + small items (`resolve_provider_alias`, `InstanceStatus::parse`, `BlockState::pending`, `send_to_conn`, `post_start_drag` stub, `TEMPLATE_PROMOTE_MARKER_V1`, never-read fields) | `state.rs:991`, `providers.rs:422`, `agents.rs:168`, `drone/types.rs:163`, `eventbus.rs:150`, `ui_tasks/drag.rs:315`, misc | ~90 |
| Machine-fixable import/mut/var warnings | 49 sites, `cargo fix --workspace` | ~60 lines |

Also: `wcore/dnd.rs` (457 LOC, all six pub fns superseded by sagas) is deletable **after the in-flight floating-pane work lands** (it's a modified file, excluded from counts).

### 2.2 False positives — fix the cfg gate, don't delete

- `agentmux-cef/src/ui_tasks/window.rs` post_* family (~250 LOC) — called only from `#[cfg(not(windows))]` sites → gate the definitions
- `agentmux-common/src/toolchain_path.rs` helpers (~90 LOC) → gate `#[cfg(unix)]`
- ~150 LOC of test-only helpers (`instance_update`, `provider_env_vars`, `inject_identity_env`, `supports_oauth`, `clear_tree_node`, etc.) → move under `#[cfg(test)]`

### 2.3 Dead-but-planned (roadmap decision needed, ~500–800 LOC)

cef host-reducer H-phase scaffolding (9+ never-constructed `HostCommand` variants with written arms, `reducer/mod.rs:260-412`); srv Phase-E fields (`state.rs:30-41`); identity API-key stub (`identity_handlers.rs:99,256`); `ContainerClient::{stop,remove}`; launcher `send_event_for_session`/`StaleSession` (test-only guard, `host_pipe/mod.rs:107,398-432`).

### 2.4 Dependencies & duplication

- **Unused deps:** `async-stream` (srv — zero refs, remove); production `tower` entry (srv — only used in tests, already a dev-dep). Minor: `which` v6 (bashwrap) vs v7 (srv).
- **`CREATE_NO_WINDOW` + `.creation_flags()` re-inlined at ~25 spawn sites** across srv/cef/launcher → extract into common.
- **Logging stack duplicated**: `srv/main.rs:1216-1319` vs `cef/lib.rs:1826-1943` near-identical (~70 LOC each) incl. a byte-identical 18-LOC `cleanup_old_logs`; bashwrap has a third variant → extract into common.
- **Verified clean:** data-dir/channel resolution, PATH enrichment, IPC types, version handling — no duplication.
- **Zero orphaned .rs files, zero stale cfg gates** (two independent passes).

**Legacy naming (Waveterm fork residue):** 412+ `wave` hits across 60 Rust files. Load-bearing and NOT unilaterally renamable: `"waveobj:update"` wire event, `wave.lock`/`wave.sock` on-disk names. Mechanically renamable: `WAVE_*` const names (values already `AGENTMUX_*`), `wstore` variable name, `wconfig` module. Tauri/Electron: all 38 hits are comments or data-migration paths (live: `~/.waveterm` migration literals, `"fromElectron"` wire arg — needs frontend lockstep).

---

## 3. Tree-shake — Frontend

Inventory: 788 files under `frontend/` (642 TS/TSX ≈ 133k LOC incl. tests; 136 SCSS ≈ 21k). Knip ran clean (caveat: its platform resolver targets win32 only — **`.darwin.*`/`.linux.*` variants are false positives; a naive knip-driven delete would break macOS/Linux builds**).

### 3.1 Verified-dead files (~4,200 LOC, high confidence — knip + manual grep both zero)

| Cluster | LOC |
|---|---|
| **`frontend/app/view/agent-def/` — entire directory (12 files)**, orphaned legacy view; only ref is a stale label at `blockutil.tsx:35` | 1,301 |
| Dead element cluster: `streamdown.tsx` (409), `emojipalette.tsx`+`emojibutton.tsx` (326), `popover.tsx` (235, only importers are dead files), `expandablemenu.tsx` (185), `multilineinput.tsx` (131), `collapsiblemenu.tsx` (84), `toggle.tsx`/`avatar.tsx`/`notification.tsx` (84) | ~1,450 |
| `suggestion/suggestion.tsx` | 336 |
| `agent/init-monitor.ts` | 262 |
| `util/ijson.ts` (253) + `util/historyutil.ts` (75) — waveterm inheritance | 328 |
| `notification/notificationpopover.tsx` + `updatenotifier.tsx` (dead chain) | 207 |
| Small files: `menu-builder.ts`, `LanStatus.tsx`, `FilterControls.tsx`, `NewAgentCard.tsx`, `nodeRefMap.ts`, dead barrels | ~550 |
| 8 orphaned SCSS companions (`avatar/collapsiblemenu/emojipalette/expandablemenu/multilineinput/notification/popover/toggle.scss`) | — |

Plus **221 unused exports + 132 unused exported types** in live files (hotspot: `store/global.ts`, ~35 dead exports).

### 3.2 npm dependencies

- **Remove (runtime):** `@dschz/solid-flow`, `@tanstack/solid-virtual` (virtualization is hand-rolled), `streamdown` (only imported by a dead file)
- **Remove (dev, verify packaging first):** `@rollup/plugin-node-resolve`, `node-abi`, `tailwindcss-animate`, `ts-node`, `tslib`
- **Add — 26 phantom deps** (direct imports of unlisted transitive packages): notably `@codemirror/{view,state,language,lint}`, `remark-parse`, `remark-rehype`, `hast-util-to-jsx-runtime`. Version-drift risk, not dead code.

### 3.3 Platform-variant duplication (~2,000 LOC recoverable)

| Triplet | Similarity | Action |
|---|---|---|
| `store/zoom.{win32,darwin,linux}.ts` | **100% identical code** (comments differ) | Merge to one file |
| `layout/lib/TileLayout.{win32,darwin,linux}.tsx` (2,543 total) | ~95–97%; real deltas 30–60 lines each | Shared core + platform extensions (~1,500 LOC saved); coordinate with in-flight layout work |
| `hook/useWindowDrag.*` | darwin/linux near-identical | Merge those two |
| `drag/CrossWindowDragMonitor.*` | Partly genuine divergence | Shared-core refactor, lower priority |
| `window/window-controls.*` | Genuinely different | Keep split (justified) |

Copy-paste pairs: `mcp/mcp-model.ts` vs `agent/agent-mcp-model.ts`, `skill/skill-model.ts` vs `agent/agent-skill-model.ts` (~75–80% identical, intentionally forked per headers — shared base extractable).

### 3.4 Misc

- Dead assets: **none** (fonts, logos, fontawesome all referenced — good state)
- Console noise: 393 console statements in production source, 141 `log`/`debug`; noisiest: `termwrap.ts` (14), `PreLaunchAuthPanel.tsx` (13, `[auth-diag]`), `termosc.ts` (12). The `debug` package is already a dependency — route through it.
- `registerBlockView()` (`block-registry.ts:45`) — exported extension point never called; registry statically eager-imports all 18 ViewModels (no lazy loading).
- Wave-era naming: 659 hits (WOS 300, waveobj 239, WaveEvent 190, wshrpc 131); modules `wos.ts`, `wps.ts`, `wps-events.ts`, `waveutil.ts`. Self-perpetuating via the `gotypes.d.ts` header instruction.

---

## 4. Code quality & hygiene

### 4.1 Markers

17 real TODOs, **0 FIXME, 0 real HACK/XXX**, 3 BUG-TRACE (all one file), 279 "Phase E" comments across 56 files (all narration), 63 "Phase 4". Zero commented-out code blocks >5 lines (four detection passes — genuinely clean).

Most important:
1. **`tabcontent.tsx:62-68`** — active `[BUG-TRACE]` IPC logging on *every* block delete, ≥4 months after the R1 fix. Remove.
2. **`identity/oauth_client.rs:71,81,91`** — Google/Microsoft/GitHub OAuth configs ship `client_id: None // TODO` (non-functional stubs in the identity subsystem).
3. **`floating_pane.rs:726`** — deferred P1: WM_CLOSE not routed through `CloseBrowser(false)` — same lifecycle area as the recent renderer-leak fixes (#1957/#1965).
4. **`CrossWindowDragMonitor.win32.tsx:363`** — failed tear-off orphans a workspace, no undo.
5. **`rpc-client.ts:96`** — RPC has no timeout; lost response = promise hangs forever.

### 4.2 Error-handling smells

- **~448 non-test `.unwrap()`/`.expect()` in agentmux-srv**, concentrated in storage (`filestore/core.rs` 35, `storage/agents.rs` 24, `sagas/log.rs` 15 — all `lock().unwrap()`): one panicking thread can cascade poison-panics through the storage layer. Launcher is nearly unwrap-free (cleanest crate).
- **14+ empty catches in TS**; worst: `store/global.ts:248-273` — four consecutive `catch (_) {}` around init listeners (a broken preload bridge is fully silent).
- **`fireAndForget` (96 call sites)** only `console.log`s rejections — state-mutating RPC failures (layout persistence, tab ops) are invisible.

### 4.3 Test coverage gaps (structural)

Zero tests: **`agentmux-srv/src/muxbus/`** (incl. PKCE — security-relevant), **`frontend/app/workspace/`** (incl. the 1,056-line floating-pane workspace under heavy churn), `TileLayout.*` (~2.5k LOC), cef top-level runtime (`lib.rs`, `state.rs`, `app.rs`, `ui_tasks/window.rs`), `server/service/workspace.rs` (1,395 — where redock logic lives). Well-covered: identity, sagas, store, agent view.

### 4.4 Consistency

- muxbus rename in **code** is complete (agentbus: 5 comment/string hits, 0 identifiers). The real drift is wave-era vocabulary: ~974 wave-prefixed identifier occurrences across 160 files.
- File naming: three conventions coexist (`app/components/` kebab vs `app/view/agent/components/` Pascal).
- License headers ~97–99.5% consistent; **invalid SPDX `Apache-2.0s`** (trailing s) in `frontend/util/{waveutil,util,focusutil}.ts:2`.

### 4.5 Config/scripts

Broken: **11 dead `tsunami:*` tasks** (`Taskfile.yml:377-466`, Go leftovers), `storybook`/`storybook:build` tasks (no config/dep exists), `npm run package:portable` → nonexistent task (`package.json:24`), phantom `docs` npm workspace (`package.json:130-132`), `scripts/verify-package.sh` (Electron/asar/wsh-era relic). CLAUDE.md/README cite `task test` — doesn't exist (tests are `npm test`).

---

## 5. Documentation audit

**913 markdown docs** (784 under `docs/`, 129 under root `specs/`), produced at ~250/month and essentially never retired (<5% archived). Sample of 30 verified against code: 9 LIVING, 13 HISTORICAL, 8 SUPERSEDED, 0 outright WRONG.

### 5.1 The retirement gap (core problem)

- Status fields written once, never re-stamped: ≥5 implemented specs still say "Draft"/"ready to implement" (`SPEC_MUXBUS_DELIVERY_HIERARCHY`, `SPEC_AGENT_CONTROL_PROTOCOL`, `SPEC_DRONE_CANVAS_NODE_EDITOR`, `SPEC_AGENT_FAILURE_DIAGNOSTICS`, `SPEC_LINUX_DOCS_UPDATE`).
- `Supersedes:` header exists in only 12 of 913 files; zero sampled docs carry a supersession banner even with an explicit replacement.
- Most misleading file found: **`docs/specs/SPEC_BACKEND_LIFECYCLE.md`** — "Status: Draft", cites removed `src-tauri/` files; its replacement (`docs/specs/process-lifecycle-v2.md`) names it, but the old file has no banner.
- Dangerous self-declared-canonical: `docs/architecture/ARCHITECTURE_AGENT_DATA_AND_CROSS_CHANNEL_2026_06_13.md` ("start here") describes pre-global-agent-promotion storage.

### 5.2 Indexes are stale

- `docs/README.md`: omits 12 of 20 subdirs, references nonexistent `docs-internal/`, states an inverted specs convention ("approved specs live in top-level `specs/`" — de facto all new specs go to `docs/specs/`: 57 vs 13 since 06-25).
- `docs/specs/INDEX.md`: links 71 of 487 specs (~15%), frozen at 2026-06-18 — misses the entire Armory/composable-model/July arc.
- Cruft: `docs/retros/` (2 strays) duplicates `docs/retro/`; 7 singleton dirs.

### 5.3 Top-level docs — verified wrong claims

| Doc | Wrong claims |
|---|---|
| **BUILD.md** (worst) | WebView2/Tauri remnants (lines 35, 401, 406); NSIS → actually Inno Setup; DEB packages → actually AppImage+Snap; "React" → SolidJS; Node 22 → `.nvmrc` 24; `--fresh` documented as meaningful (no-op); log paths that don't exist |
| **CLAUDE.md** | Widget table lists 9 of 17 widgets, mislabels 5 as Pinned (real pinned: 4 — agent, swarm, drone, warden); Settings wrongly in "Not widgets" (in-app Settings pane shipped, PR #1792); stale `AGENTMUX_DEV=1 → ~/.agentmux-dev` (code is branch-keyed); cites nonexistent `task test` |
| **README.md** | Same widget drift (9 vs 17, "every widget pinned"); Settings claim stale; Node 22; `task test` |
| **VERSION_HISTORY.md** | Healthy top (0.50.0 invariant holds); tail: stranded "Latest Version: 0.34.0" at line 1817, line-2443 "Version Bumps" section contradicts the changesets workflow |

### 5.4 Naming

The muxbus rename took in docs: only **1 post-June violation** (`SPEC_MESSAGING_INTEGRATIONS_PLAN_2026_06_24.md:195` "AgentBus cloud relay" — one-word fix). The 10 other agentbus docs are ≤05-25 protected historical files (per CLAUDE.md rule — untouched).

---

## 6. Cross-repo triangulation (agentmux-cloud, agentmux-docs)

### 6.1 The headline contradiction (public positioning)

`agentmux-docs internals/interagent-comms.md:92`: *"AgentMux does not run a relay — you bring your own (the open-source `@agentmuxai/muxbus-server`)"* — while `agentmux-cloud` ships a **proprietary metered relay** (Fastify server billing `jekt_messages` per message, Stripe, free tier, `upgrade_url: cloud.agentmux.ai/billing`, `SPEC_FREE_TIER_PRICING_2026_06_21.md`). One of these is false. Decide the story, fix the docs.

### 6.2 Jekt trust rules — three diverging copies

| Aspect | main `CLAUDE.md` | main `sanitize.rs` (host tier) | cloud `muxbus/server/src/index.ts` (WAN) |
|---|---|---|---|
| Marker fields | abbreviated | full incl. `TS=` | **missing `TS=`** |
| Keyword list | 16 (missing `webhook secret`, `auth key`) | 19, careful | same 19 + `apiKey` |
| Matching | prose | **whole-word** for pat/token/secret/… | **naive substring** — `'pat'` matches "path", `'token'` matches "tokenizer" → WAN false-positive escalations |

All three lists still contain `trust center`; **none contain `armory`** (the feature's actual name since PR #1917). Canonical source should be the main-repo spec (`SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md`); cloud should port the whole-word matcher.

### 6.3 agentmux-docs currency

Frozen at ~2026-06-23 (prod deploy pinned at 0.1.4) while the product shipped v0.50.x, the Armory rename, Bundle rename, Armory MCP/Skills tabs, and jekt markers. Concretely stale:
- "Trust Center" everywhere (trust-center.md, sidebar `astro.config.mjs:122`, auth/glossary/identity/memory/pane-types/agent-app-api) — renamed **Armory**
- `pane-types.md` lists 9 pane types; `widgets.json` registers **17** (missing Discord/Slack/Telegram/WhatsApp/Teams/Toolchain/Armory/Settings)
- Bundle semantics wrong (`glossary.md:40` says bundle includes provider/model; it's provider-agnostic since PR #1918)
- `security/trust-model.md` has **zero coverage of the jekt trust-marker system** — the biggest security-doc gap
- Version waypoints stuck at v0.40–v0.46 ("shipped in v0.46. Future phases will…")
- Verified-accurate: settings keys, install channels, build prereqs — the corpus is well-built, just frozen.

### 6.4 agentmux-cloud

`README.md` on origin/main still says *"Status: exploratory. No code yet"* — while the repo contains a deployed Fastify server, CDK infra (Cognito/DynamoDB/WebSocket), GitHub webhook consumer, and a client package. Badly stale.

### 6.5 Proposed "where knowledge lives" rule

1. **agentmux-docs** = everything a user/operator reads. Never document a feature only in `agentmux/docs/`.
2. **agentmux `specs/` + `docs/`** = engineering truth + canonical wire-format specs for anything both repos implement (jekt marker, delivery hierarchy).
3. **agentmux-cloud** = relay/consumer implementation + business docs; protocol semantics imported from main-repo specs, never redefined.
4. **Renames aren't done** until the docs repo and all keyword/string lists are swept — add to the rename-spec checklist.

---

## 7. Consolidated action plan

### Tier 0 — Quick wins (hours each, no design decisions)

| # | Action | Evidence |
|---|---|---|
| 1 | Remove active BUG-TRACE logging | `frontend/app/tab/tabcontent.tsx:62-68` |
| 2 | `cargo fix --workspace` (49 mechanical warning fixes; skip in-flight files) | §2.1 |
| 3 | Delete dead Taskfile/`package.json` entries: 11 `tsunami:*` tasks, `storybook`, `package:portable` script, phantom `docs` workspace; delete `scripts/verify-package.sh` | §4.5 |
| 4 | Fix invalid SPDX `Apache-2.0s` (3 files) | §4.4 |
| 5 | Remove unused deps: `async-stream`, prod `tower` (Rust); `@dschz/solid-flow`, `@tanstack/solid-virtual`, `streamdown` (npm) | §2.4, §3.2 |
| 6 | One-word muxbus fix in `SPEC_MESSAGING_INTEGRATIONS_PLAN_2026_06_24.md:195` | §5.4 |
| 7 | Delete stale progress comments: `reducer.rs:86` "4 of 11", update `SPEC_STRONG_REDUCER_AUTHORITY` status note | §1.4 |

### Tier 1 — Dead-code deletion PRs (a day each, high confidence)

| # | Action | LOC |
|---|---|---|
| 8 | Delete `frontend/app/view/agent-def/` + stale label | 1,301 |
| 9 | Delete frontend dead element cluster + 8 SCSS + `suggestion.tsx`, `init-monitor.ts`, `ijson.ts`, `historyutil.ts`, notification chain, small files | ~2,900 |
| 10 | Delete `agentmux-srv/backend/userinput.rs` | 381 |
| 11 | Delete superseded wcore mutation layer (zero-ref subset); `wcore/dnd.rs` after ghost fix lands | ~395 (+457) |
| 12 | Delete `wndproc.rs` frameless cluster, `build_config_files_with_bus`, non-`_unique` upserts, cef AppState leftovers, small dead items | ~340 |
| 13 | cfg-gate false positives (`ui_tasks/window.rs` `#[cfg(not(windows))]`, `toolchain_path.rs` `#[cfg(unix)]`); move test-only helpers under `#[cfg(test)]` | kills ~12 warnings + ~150 LOC out of shipping binary |

### Tier 2 — Docs sprucing (mostly mechanical)

| # | Action |
|---|---|
| 14 | Rewrite BUILD.md wrong claims (WebView2, NSIS→Inno, React→SolidJS, Node 24, `--fresh`, log paths → point at muxlog) |
| 15 | Regenerate widget tables in CLAUDE.md + README.md from `widgets.json` (17 widgets, 4 pinned); fix Settings, `AGENTMUX_DEV` bullet, `task test` |
| 16 | Refresh `docs/README.md` (drop `docs-internal/` ghost, add 12 missing dirs, state the real specs policy: root `specs/` frozen, new specs → `docs/specs/`); regenerate `docs/specs/INDEX.md` (script it from filename dates + Status headers) |
| 17 | Supersession banners: `SPEC_BACKEND_LIFECYCLE.md`, `docs/specs/archive/SPEC_TRUST_CENTER_2026_06_15.md`, `ANALYSIS_AGENT_APP_API_OPEN_IN_EDITOR`, `openclaw-agent-runtime.md`, `single-instance-new-window.md`; as-of banner on `ARCHITECTURE_AGENT_DATA_AND_CROSS_CHANNEL` |
| 18 | Re-stamp the 5 implemented-but-"Draft" specs; adopt closed Status vocabulary (`draft/approved/implemented/living/historical/superseded`) + `Superseded-by:` convention for new docs |
| 19 | VERSION_HISTORY.md tail cleanup (line 1817 stray, line 2443 bump-cli instructions); merge `docs/retros/`→`docs/retro/`, fold singleton dirs |

### Tier 3 — Cross-repo sync (needs a product decision on #20)

| # | Action |
|---|---|
| 20 | **Decide the relay story** (open-source self-hosted vs cloud.agentmux.ai paid) and fix `interagent-comms.md` + cloud README accordingly |
| 21 | Sweep Trust Center→Armory + Bundle semantics + 17-widget list through agentmux-docs; document jekt trust markers in `security/trust-model.md`; then **redeploy the docs site** (prod pinned at 0.1.4/06-23) |
| 22 | Unify jekt keyword list + matcher: declare main-repo spec canonical, port whole-word matching to cloud `index.ts`, add `armory` keyword everywhere, add `webhook secret`/`auth key` to CLAUDE.md |
| 23 | Rewrite `agentmux-cloud/README.md` ("no code yet" → actual contents); adopt the "where knowledge lives" rule in both repos' CLAUDE.md |

### Tier 4 — Structural (planned engineering work, in dependency order)

| # | Action | Why this order |
|---|---|---|
| 24 | **Finish the layout single-writer cutover** (frontend intent-flip, `UpdateObject`→`LayoutSetTree` reroute, retire 7 wcore writers, add CAS) | Unblocks Pillar 1 → saga collapse; ends the split-brain that generates floating-pane bug churn |
| 25 | **Pillar 2 Stage 2** (consume `reconcile_quit`, delete inline gate) then Stage 3 (demote orphan_reconcile/WRR to executors) | Ends the quit split-brain — the top PR-churn generator |
| 26 | **Execute agents Phase 3c** (drop legacy tables, delete `dual_write.rs` + `agents_consolidate.rs`, ~1,600 LOC) | Cheapest of the three cutovers; removes a whole failure mode |
| 27 | Consolidate platform triplets: merge `zoom.*` (trivial), extract TileLayout shared core (~1,500 LOC; after in-flight layout work lands) | Most-touched render path; every fix currently ×3 |
| 28 | Extract shared logging + `cleanup_old_logs` + `CREATE_NO_WINDOW` helper into agentmux-common | ~200 LOC + consistency |
| 29 | Add wire-contract drift protection: contract test or codegen for `gotypes.d.ts` vs `obj.rs`/`rpc_types.rs` | Removes the largest silent-drift surface |
| 30 | Frontend hardening: RPC timeout (`rpc-client.ts:96`), un-silence `global.ts:248-273` catches, surface `fireAndForget` failures; add tests for `muxbus/` (PKCE) and `floating-pane-workspace.tsx` | Highest-risk hygiene gaps |
| 31 | Roadmap decision on dormant scaffolding: cef H-phase HostCommand variants, identity API-key stub, ContainerClient stop/remove — land the dispatching PRs or prune | ~500–800 LOC pool |
| 32 | Next-tier god-file modularization specs: `persist_subscriber.rs`, `cef/lib.rs::run()`, `mcp/main.rs`, `identity/resolver.rs`, `app-init.ts`, `AgentLaunchModal.tsx`, `tabbar.tsx`, `floating-pane-workspace.tsx` | The >2.2k tier program worked; repeat it |

### CI guards to keep it clean

- `cargo machete`/`cargo udeps` + `knip` in CI (with the platform-resolver caveat configured)
- `cargo check --workspace --all-targets` on at least one non-Windows target (stops platform/test false positives masking real dead code)
- Treat new file-scope `#![allow(dead_code)]` as review-blocking (52 exist in srv today)
- Release-consistency check already exists; add a docs-index freshness check if INDEX.md becomes scripted

---

## 8. Bright spots (worth preserving)

- Strict star dependency topology; zero store→view cycles; single wire-protocol source
- The >2.2k god-file modularization program completed all four targets — the playbook works
- Zero FIXMEs, zero commented-out code, zero orphaned .rs files, zero stale cfg gates, zero dead assets
- ~97–99.5% consistent license headers; launcher crate nearly unwrap-free
- Doc culture: 83% of specs carry Status/Date headers; muxbus rename fully took in code and June+ docs
- CI workflows all resolve; isolation invariants (I1–I6) documented and enforced
