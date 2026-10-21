# ANALYSIS evidence: Rust dead-code audit (static, read-only) - AgentMux v0.59.35 snapshot

**Date:** 2026-10-20
**Status:** analysis. The evidence behind docs/analysis/ANALYSIS_DEAD_CODE_AND_TREE_SHAKING_2026_10_20.md (read-only, static: nothing was compiled or run). Re-verify an item before acting on it.
**Note:** where this file mentions `scratch/...` helper scripts or listings (`deps.py`, `allows.py`, `allowed.txt`), those were throwaway analysis files and are not kept; the lists are reproducible with ripgrep and `knip` as the text describes.


Snapshot: a read-only extract of origin/main (2026-10-20). 8 crates, 1283 `.rs` files, ~263k lines (incl. ~100k lines of tests). No cargo run of any kind; every claim below is from ripgrep / small python token-index scripts (throwaway, not kept; see the note above).

Confidence tags: CONFIRMED = the identifier appears nowhere outside its own definition (and, where stated, its own tests) in crates/, frontend/, scripts/, schema/, sdk/, .github/, Taskfile, packaging. LIKELY = needs a human glance (name collisions, planned/unwired code, platform cfg).

Method for item-level findings (section 3): token index of every identifier in every `.rs` file, split into production vs `#[cfg(test)]`/tests-file regions; an item is "unreferenced" when every occurrence of its name is either a definition or inside the span of another unreferenced item (fixpoint, 7 rounds, so call chains of dead code are found). Trait-impl methods, `wrap_*!` CEF macro bodies, `#[test]`, `#[no_mangle]`, `extern` were treated as live roots. Name-based, so it is conservative: a name shared with a live item is never flagged (false negatives possible, false positives rare and spot-checked). Then every flagged name was re-grepped with `rg -w` over the whole repo (excluding node_modules, docs).

Important framing: srv, launcher, mcp, bashwrap are bin-only crates, so rustc already warns on any unreferenced item. Dead code that survives is therefore almost entirely code hidden behind `#[allow(dead_code)]` (or reachable only from such code). Dead code is stripped by the linker, so the saving is source lines and compile time, not shipped binary KB (negligible).

---------------------------------------------------------------------------
## 1. Cargo.toml dependencies, features, members

Result: **no unused dependency found.** CI already runs `cargo machete` (`.github/workflows/ci-pr.yml:493-499`), and a per-dependency check agrees.

Evidence: `scratch/tools/deps.py` parsed every `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]` and `[target.*]` section of all 8 manifests and counted `\b<crate_ident>\b` hits in every `.rs` of that crate. Every dep has at least one real use; the low-count ones were opened and confirmed:
- cef: `whoami` (commands/platform.rs:166), `json_comments` (platform.rs:374), `tower-http` (ipc.rs:28-29), `tracing-subscriber` (logging.rs:14), `rfd` (platform.rs:1045+), `winres` (build.rs, ignored in `[package.metadata.cargo-machete]`).
- common: `same-file` (checkout.rs:120), `thiserror` (errors.rs:16), `psl` (popup_rules.rs:102), `hmac` (jekt_sign.rs:38), `ts-rs` is `optional`, enabled by `ts` feature which srv turns on (`srv/Cargo.toml:agentmux-common features=["ts"]`).
- srv: `memchr` (history/record.rs:241), `tracing-log` (bootstrap/logging.rs:20), `curve25519-dalek` (server/jekt_seal.rs:33), `quick-xml`/`calamine`/`lopdf` (attachments/extract.rs), `hyper-util` (lan_listeners.rs:522), `cron` (server/cron.rs:18), `trash` (fs_ops/trash_worker.rs:66), `objc2` (macOS only; zero hits but `objc2_foundation` is used at trash_worker.rs:176 and objc2 is in the machete ignore list with a comment "drop once a macOS build proves objc2-foundation alone is enough").
- launcher / bashwrap / mcp / remote / procstats: all deps used (bashwrap `chrono` precompact.rs:147 is its only use).

Features
- Declared: cef `sandbox` (default), `test-authfile`, `patched-libcef`; common `ts`. All four are enabled somewhere: `test-authfile` in `ci-nightly-build.yml:228-230`; `patched-libcef` in `Taskfile.yml:552`; `sandbox` by default and `cef/sandbox`; `ts` by srv.
- `#[cfg(feature = "...")]` uses: only `ts` (22), `sandbox` (10), `test-authfile` (2), `patched-libcef` (2); none undeclared.
- No `[workspace.dependencies]` (only `[workspace.package]`), so versions are repeated per crate by hand.

Workspace members: all 8 are built/packaged (Taskfile.yml:499/527/552, scripts/package-portable.sh, package-macos.sh, stage-linux-runtime.sh, build-remote-helpers.sh for `agentmux-remote`, `procstats` via srv/remote/cef/bashwrap). No unused path dependency. `crates/procstats/examples/memtree.rs` (675 lines) is built by `scripts/perf/memory-lab/run.mjs:122`. `[[bin]]` entries: one per crate, all built.

Minor findings (LOW):
| id | file:line | claim | conf | action | risk |
|---|---|---|---|---|---|
| D1 | `launcher/Cargo.toml:41-45` | Comment says chrono `serde` feature is for "the vacuum cutoff parameter" (`vacuum_older_than`, deleted in the saga collapse). No `DateTime` is (de)serialized in launcher (`rg DateTime crates/launcher/src` -> only `tray/notify_menu.rs` Local); only `Utc::now` in `saga/log/mod.rs:37` and `Local` in notify. | LIKELY | drop `"serde"` from `chrono` features (feature unification: bollard's `chrono` feature in srv may still enable it workspace-wide, so no size win guaranteed) and fix comment | none |
| D2 | `launcher/Cargo.toml:57-60` | `thiserror` only for `LogError` in `saga/log/mod.rs:60`; comment still says "SQLite + serde_json + io error wrapping" | LIKELY | keep dep (srv/common use it); fix comment | none |
| D3 | `srv/Cargo.toml` `[dev-dependencies]` | `serde_json = "1"` and `libc = "0.2"` duplicate the normal deps exactly (no extra features) | CONFIRMED redundant | delete the two dev entries (tokio/tower/reqwest/proptest dev entries add features, keep) | none |
| D4 | multiple | Duplicate versions of directly-declared deps in Cargo.lock: `axum` 0.7 (srv) vs 0.8 (cef) (+axum-core, matchit), `which` 7 (srv) vs 6 (bashwrap), `png` 0.17 (launcher build-dep) vs 0.18 (launcher runtime dep), `thiserror` 1.x (common/launcher/srv) vs 2.x (transitive), `dirs` 5 (4 crates) vs 6 (transitive). 90 duplicated package names overall (windows-* family, nix x4, syn x3, ...). | LIKELY | unify axum (cef -> 0.7 or both -> 0.8), `which` 6->7, `png` build-dep; potential saving: a few hundred KB in cef/srv and several crates to compile (not measured) | axum 0.7->0.8 is a real migration for srv (path syntax `/:id` -> `/{id}` seen in `server/routes.rs:539`) |

---------------------------------------------------------------------------
## 2. `#[allow(...)]` inventory

Totals (all `allow(` attributes in `crates/**/*.rs`): **365** = srv 267, cef 50, launcher 34, common 8, procstats 4, bashwrap 1, remote 1, mcp 0.
By lint: `dead_code` 229 (+1 `dead_code, unused_imports`), `clippy::too_many_arguments` 35 (srv 28, common 7), `unused_imports` 15, `unreachable_code` 6, `unused_mut` 5, `clippy::permissions_set_readonly_false` 3, `unused_variables` 3, `deprecated` 2, `clippy::type_complexity` 2, one each of `non_snake_case`, `non_camel_case_types`, `clippy::suspicious_open_options`, `clippy::result_large_err`.
Crate-wide `#![allow(dead_code)]` (13 files, all srv except one launcher): `srv/src/backend/{base,docsite,ijson,service,schema,shellexec,userinput,rpc_types/mod,wcore/event,wcore/block}.rs`, `srv/src/backend/storage/filestore/ijson.rs`, `srv/src/registry/mod.rs` (`dead_code, unused_imports`), `launcher/src/host_pipe/connection.rs`. Platform-conditional crate-level: `srv/src/backend/lan_firewall.rs:30`, `cef/src/linux_sandbox.rs:26` (legitimate).

Classification of the 229 `dead_code` allows (script `scratch/tools/allows.py`, then name-index):
| class | srv | cef | launcher | other |
|---|---|---|---|---|
| module-wide `#![allow]` | 13 files | 1 | 1 | - |
| platform `cfg_attr(not(os), allow(dead_code))` (keep, justified) | 18 | 17 | 15 | remote 1 |
| item with no reference anywhere | 29 | 1 | 1 | - |
| item used only by tests (test-only) | 35 | 1 | 0 | - |
| item referenced elsewhere (allow stale, or only used from other dead code, or name collision) | 63 | 13 | 6 | procstats 4 |
| field / variant / impl / mod | 46 | 8 | 9 | - |
Detailed lists: `scratch/tools/allowed.txt` (121 rows, file:line kind name, prod/test/other ref counts).

### 2a. Items under allow with NO reference anywhere (CONFIRMED; each name re-grepped with `rg -w` over repo, 1 file hit = its own file)
(file:line = item line; sizes are the item span)
- `srv/src/backend/wcore/event.rs:13` `send_mux_obj_update` (46 lines; whole file is this fn + imports).
- `srv/src/messaging/slack/rest.rs:173` `post_response_url` (23); `srv/src/messaging/mod.rs:23` `InboundMsg` (15).
- `srv/src/backend/shellexec.rs`: consts `BLOCK_FILE_CACHE:37`, `BLOCK_FILE_ENV:38`, `DEFAULT_HTML_MAX_FILE_SIZE:44`, `MAX_INIT_SCRIPT_SIZE:47`; fn `signal_exit:253`.
- `srv/src/backend/base.rs`: consts `MUX_DEV_VITE_ENV:21`, `MUX_JWT_TOKEN_ENV:22`, `MUX_SWAP_TOKEN_ENV:23`, `REMOTE_DOMAIN_SOCKET_BASE_NAME:29`, `REMOTE_MUX_HOME_DIR_NAME:32`, `REMOTE_FULL_DOMAIN_SOCKET_PATH:33`; fns `get_build_time:56`, `get_mux_app_bin_path:161`, `ensure_mux_config_dir:196`, `ensure_mux_presets_dir:201`.
- `srv/src/backend/blockcontroller/mod.rs`: `META_KEY_CMD_SHELL:86`, `META_KEY_CMD_JWT:129`, `META_KEY_CMD_INIT_SCRIPT{,_BASH,_ZSH,_FISH,_PWSH}:137-145` (7 consts); `stop_all_controllers:677` (8 lines). `shell/controller.rs:323` `should_close_on_exit_force` (25 lines). `blockcontroller/process_tree.rs:13` `MAX_PIDS_PER_BLOCK` (2 lines; only a doc-comment mention at `backend/sysinfo.rs:146`; CONFIRMED unreferenced).
- `srv/src/backend/mps.rs`: `EVENT_USER_INPUT:58`, `EVENT_ROUTE_GONE:73`, `FILE_OP_CREATE:157`, `FILE_OP_INVALIDATE:166`, `has_scope:201`; `obj.rs:183` `UPDATE_TYPE_DELETE` (+ `UPDATE_TYPE_UPDATE`).
- `srv/src/backend/eventbus.rs:138` `wait_for_connection` (18); `lan_discovery.rs:1308` `peer_count`; `process_tracker/windows.rs:67` `assign_inherent` (8, Windows only).
- `srv/src/backend/reactive/poller.rs:103,115` `record_error`, `set_running`; `reactive/handler.rs:231,723,1794,1928` `set_include_source`/`update_last_seen` (two copies each, once for each of two `impl` blocks); `reactive/types.rs:544` `AgentListResponse`, `:703` `PendingInjection`.
- `srv/src/backend/rpc/engine.rs:114` `get_source`.
- `srv/src/backend/storage/filestore/core.rs:1034` `start_flusher` (26); `storage/store.rs:521` `exists_raw` (12), `:614` `delete_by_otype` (15).
- `srv/src/backend/userinput.rs:124` `SSH_PROMPT_TIMEOUT`; `wconfig/mod.rs:33,35` `CONNECTIONS_FILE`, `PROFILES_FILE`; `rpc_types/mod.rs:112` `is_rpc_request`.
- `launcher/src/host_pipe/connection.rs:29` `ConnectionTelemetry` (22 lines; **the whole 46-line file is this unused struct + `#![allow(dead_code)]`; `mod connection;` is at `host_pipe/mod.rs:62`**).
- `launcher/src/upgrade.rs:145` `run_migration_upgrade` (20 lines) - annotated "No caller until phase 1b of SPEC_FAST_STARTUP_UPGRADE_OWNS_MIGRATIONS_AND_UPDATES_2026_09_15.md"; its helpers (`run_migrate`, `srv_spawner.rs:226` comment, `GRACEFUL_QUIESCE_TIMEOUT` is also used by a test). LIKELY intentional scaffolding, not removable without product decision.
- `cef/src/memory_pressure.rs:153` `with_thresholds` (8).
Suggested action: delete. Risk: low (compiler enforces; consts have zero greps).

### 2b. Items under allow used only by tests
Examples (full list `scratch/tools/allowed.txt`, rows with TESTONLY): `srv/.../blockcontroller/app_server_protocol.rs` `fork_thread:460, read_thread:491, list_threads:514, archive_thread:531, compact_thread:549, read_account:572, logout_account:588, login_account:599, cancel_login:616, steer_turn:634, interrupt_turn:664, respond_server_request:782` (12 methods, ~305 lines; each annotated "Unwired until SPEC_CODEX_APP_SERVER_FIRST_CLASS_PROVIDER_2026_09_12.md §15 Slice 6" -> planned, LIKELY keep); `storage/conversation_trust_grants.rs:68,88,108` (3 fns, 65 lines); `storage/agent_tokens.rs:151` `agent_token_load` (16); `storage/agent_wan_keys.rs:113` `agent_wan_key_ensure` (41; callers found are `agent_config.rs:2628`, `agent_tokens.rs:367` and its own tests, all looked test-side; LIKELY test-only, verify); `reactive/handler.rs:1877` `try_register_agent_with_nonce` (77 lines, `#[cfg_attr(not(test), allow(dead_code))]`); `reactive/sanitize.rs:364` `wrap_jekt_message` (143 lines; only tests/doc comments call it; `wrap_jekt_message_with_files` is the live one); `reactive/audit_store.rs:92` `read_all` (annotated "used by the tests; no production reader yet"); `rpc/engine.rs` `register_stream_handler, set_auth_token, send_command_no_response`; `eventbus.rs:294` `send_to_tab`; `dock_snapshot.rs:126` `evict_block`; `wconfig/loader.rs:249` `expand_env_vars` (35); `wconfig/types.rs:876` `WebhookConfigType` (22); `wcore/block.rs:72` `resolve_block_id_from_prefix` (19); `service.rs:139` `get_method_meta` (261 lines, a Go-service method table) + `MethodMeta`, `SERVICES`, `is_valid_service`; `base.rs` `replace_home_dir, client_arch, get_system_summary, determine_lang, is_dev_mode, get_domain_socket_name`; `cef/src/browser_panes/media_grants.rs:108` `pane_grants`.
Suggested action: delete together with their tests (or `#[cfg(test)]` them). Test-only total from the fixpoint pass: ~74 srv items / ~1330 lines (includes the test-only chains below).

### 2c. Module-wide `#![allow(dead_code)]` files - what they hide (name-index per file)
| file | prod / test lines | status |
|---|---|---|
| `srv/src/backend/userinput.rs` | 203 / 180 | CONFIRMED whole module unreferenced (see F1) |
| `srv/src/backend/ijson.rs` | 465 / 326 | CONFIRMED dead chain (see F2) |
| `srv/src/backend/storage/filestore/ijson.rs` | 125 / 0 | CONFIRMED dead chain (F2) |
| `srv/src/backend/shellexec.rs` | 363 / 140 | partly live: `ConnInterface`, `MockConn` (used by `blockcontroller/shell/*` incl. tests), `ShellProc` live; ~15 consts/fns dead or test-only (list in 2a/2b); `MockConn` is a test double living in production code (`shell/tests.rs:20` uses it) |
| `srv/src/backend/service.rs` | 451 / 167 | live types (`WebCallType`, `WebReturnType`, `CloseTabRtnType`, `get_arg`, `get_optional_arg` used by `server/service/*`); `get_method_meta` (261 lines) + `MethodMeta` + `SERVICES` + `is_valid_service` test-only; the struct `UIContext` is also defined in `backend/obj.rs:189` (duplicate) |
| `srv/src/backend/base.rs` | 550 / 324 | live: lock/data-dir/safe_join; ~19 consts/fns dead or test-only (2a/2b) |
| `srv/src/backend/schema.rs`, `docsite.rs` | 65 / 100, 86 / 26 | live; only `schema_dir_exists` and `is_safe_path` test-only (allow could be per-item) |
| `srv/src/backend/wcore/{event,block}.rs` | 46, 90 | event.rs entirely dead (2a); block.rs one test-only fn |
| `srv/src/backend/rpc_types/mod.rs` (covers 24 submodules, 11k lines) | - | 36 unreferenced items, ~190 lines (see F3); the blanket allow also hides unused `pub use X::*` surfaces |
| `srv/src/registry/mod.rs:18` `dead_code, unused_imports` | 61 | no dead item found by the index in `srv/src/registry/**` -> allow looks removable (LIKELY; needs one compile to confirm) |
| `launcher/src/host_pipe/connection.rs` | 46 | CONFIRMED dead file (2a) |

### 2d. Other lints
- `unused_imports` x15 on `pub use` re-exports: `backend/reactive/mod.rs:63,65,67,72`, `backend/storage/filestore/mod.rs:44,47,50,52`, `backend/storage/mod.rs:58`, `backend/storage/agents/mod.rs:49`, `backend/subagent_watcher/mod.rs:67,73`, `backend/wcore/mod.rs:18`, `identity/mod.rs:47`, `reducer/layout.rs:12`. Each re-exported name does occur elsewhere by name (e.g. `Poller` 19 hits, `AgentHistory` 17), but whether callers use the re-export path or the full path is unknown. LIKELY partly stale; needs one `cargo check` after removing the allows.
- `unreachable_code` x6 (`cef/src/commands/drag.rs:248,293`, `platform.rs:237,294`, `window/panel.rs:181`, `launcher/src/notify/mod.rs:138`): platform stub arms, justified.
- `deprecated` x2: **duplicate code** `v6.to_ipv4()` at `srv/src/backend/lan_discovery.rs:204` and `srv/src/drone/executor/blocks/api.rs:139`.
- `unused_mut`/`unused_variables`/`non_*` : cfg-specific or FFI, justified.
- `clippy::too_many_arguments` x35: signal for refactor, not dead code.

---------------------------------------------------------------------------
## 3. Unreferenced items, orphans, duplicates

### F1 `srv/src/backend/userinput.rs` - whole module dead (CONFIRMED, HIGH value)
383 lines (203 prod + 180 test). `pub mod userinput;` at `backend/mod.rs:149`. Evidence: `rg 'userinput|UserInputHandler|UserInputRequest|UserInputResponse' crates/srv/src` outside the file -> only a string `("userinput","SendUserInputResponse")` in `server/service/misc.rs:16` whose body is `// Accept but drop — user input routing not yet wired` and a comment in `app_api/pane.rs:191`. The Rust types (`UserInputRequest`, `UserInputResponse`, `UserInputHandler`, `wait_for_response`) are never constructed. Header says "Port of Go's pkg/userinput". Related (other reviewers): the frontend still has `frontend/app/modals/userinputmodal.tsx:23-46` + `store/services.ts:119` calling `UserInputService.SendUserInputResponse` and `UserInputRequest` in `frontend/types/srv-types.d.ts` - i.e. a frontend call into a stub that drops the message. Action: delete `userinput.rs` (and its `mod` line); decide separately whether the frontend modal + stub arm are also dead. Risk: low for the Rust side.

### F2 ijson stack - transitively dead (CONFIRMED, test-only chain, HIGH value)
`srv/src/backend/ijson.rs` (791 lines, 465 prod) is referenced only by `srv/src/backend/storage/filestore/ijson.rs:102` (`crate::backend::ijson::compact_ijson`), which is the body of `FileStore::compact_ijson`, called only by `FileStore::append_ijson` (same file:30), which is called only from `storage/filestore/tests.rs:599,620` (`rg 'append_ijson|compact_ijson' crates` -> 0 production callers). `FileOpts.ijson` / `ijsonbudget` (`filestore/types.rs:19,21`, `rpc_types/misc.rs:111,113`) are never set to true in production (`rg 'ijson' crates frontend/app scripts` only hits those fields and `term.scss:70 .ijson iframe`). Whole feature (Go "incremental JSON" files) is dead. Savings: ~590 prod lines + ~326 lines inline tests + ~80 lines in `filestore/tests.rs` (tests at :584-660) + the two option fields. Action: delete both files, the `mod`/`pub mod` lines (`backend/mod.rs:66`, `filestore/mod.rs:18`), the 2 fields and the ijson tests. Risk: low-medium (FileOpts fields may be in ts-rs exported types -> regenerate `frontend/types/rpc`).

### F3 Go-port leftover wire types in `srv/src/backend/rpc_types/` (CONFIRMED unreferenced in Rust, MEDIUM)
36 items, ~190 lines, all under the blanket `#![allow(dead_code)]` of `rpc_types/mod.rs`: consts `commands.rs:9-29` (`MAX_FILE_SIZE, MAX_DIR_SIZE, FILE_CHUNK_SIZE, DIR_CHUNK_SIZE, LOCAL_CONN_NAME, RPC_TYPE_*, CREATE_BLOCK_ACTION_*`) and `:775-776` `CLIENT_TYPE_*`; structs `misc.rs`: `CommandAuthenticateRtnData:20, CommandAuthenticateTokenData:32, CommandDisposeData:38, CommandResolveIdsData:44, CommandResolveIdsRtnData:52, FileData:66, CommandWaitForRouteData:132, BlockInfoData:140, MuxInfoData:152, WorkspaceInfoData:167, MuxNotificationOptions:193, CommandVarData:233, CommandVarResponseData:247, RemoteInfo:279`; `block.rs:29,37,59,65` (`CommandMessageData, CommandCreateBlockData, CommandDeleteBlockData, CommandBlockSetViewData`); `agent.rs:73 CommandAgentStreamData`; `mod.rs:112 is_rpc_request`.
Evidence: `rg -w <name>` -> only the definition (+ same-named TS types in `frontend/types/srv-types.d.ts` which is a Go-era `wshrpc.*` mirror; `CommandMessageData` is used in `frontend/app/store/rpc-client.ts:151`; `CommandDeleteBlockData` has a generated `frontend/types/rpc/CommandDeleteBlockData.ts` and re-export in `rpc-api/block.ts:43` so it is TS-exported; check before removing those two). Action: delete the Rust structs/consts; separately prune `srv-types.d.ts`. Risk: low except the 2 TS-visible types.

### F4 Other transitive / test-only clusters (from the fixpoint pass; details in `scratch/tools/dead.json`)
Totals after dropping false positives (procstats example-only fns, local consts, ts-rs assoc types): **97 items / ~570 lines with zero references** and **81 items / ~1390 lines referenced only by tests** (srv: 93 + 74 items; common 1+4; launcher 2+1; cef 1+1; mcp 1 test-only). Biggest by lines: `service.rs get_method_meta` 261, `reactive/sanitize.rs wrap_jekt_message` 143, `reactive/handler.rs try_register_agent_with_nonce` 77 (+ duplicate `register_agent*` pair), `app_server_protocol` 12 methods 305, `wcore/event.rs` 46, `agent_wan_key_ensure` 41, `ijson` cluster.
Lib-crate pub items unreferenced from any workspace crate (common/procstats/remote, all LOW): `common/src/api_types.rs:943 UiBrowserOpenResponse` (CONFIRMED, 6 lines), `common/src/jekt_sign.rs:566,569 WAN_INSTANCE_ID_LEN/WAN_KEY_FP_LEN` (tests only), `common/src/runtime_mode.rs:166 dir_slug` (28 lines, tests only), `common/src/transcript_request.rs:50,97,118 to_message x2, parse_transcript_response` (tests only), `procstats` `write_bytes`/`storage_write_bytes` (used only by `examples/memtree.rs` and the JS memory lab: keep), `CHECKOUT_DIR` consts (checkout guard, keep).

### F5 Orphan files, assets, cfg, stubs (all clean)
- Orphan `.rs` files (not declared by any `mod`/`#[path]`): **none** (`scratch/tools/orphans.py`, 1283 files).
- `include_str!/include_bytes!` assets: all used (`config/{tool-catalog,starter-mcp-servers,starter-skills,widgets}.json`, `agent-seed.json`, `operator-config-seed.json`, `ambient/title_corpus.json`, `shellintegration/*`, `testdata/update-portable-0.59.27.7z` by `update_unpack.rs`/`update_download.rs` tests, `launcher/resources/{brain,menubar-template}.png`, `cef/resources/win/agentmux.ico`, `browser_api/scripts/{click_label,query}.js`). `srv/tests/fixtures/*` are used by `tests/subprocess_io.rs` / `test_support.rs:68`.
- Platform cfg: only `windows` (766 uses), `macos` (224), `linux` (183), `android`/`freebsd` (only inside `any(...)` fallbacks at `common/src/checkout.rs:89`, `srv/.../trash_worker.rs`, `backend/sysinfo.rs:94`), `x86_64`. No cfg for a never-built platform and no undeclared feature. Nit: `trash_worker.rs:289-296` `path_key` allow for a target set that never occurs.
- Commented-out code: scanned all `//` runs >=4 lines with >=40% code-like lines -> **none**.
- Empty/stub modules: none (smallest files are `mod` aggregators). Nearly-empty: `launcher/src/host_pipe/connection.rs` (above), `srv/src/backend/wcore/event.rs`.
- Binaries not built: none.

### F6 Duplicate helpers across crates (CONFIRMED identical bodies; `scratch/tools/dup.py`)
| helper | locations | note |
|---|---|---|
| `fnv1a_64` | `common/src/runtime_mode.rs:337` and `launcher/src/hash.rs:30` | common's comment says "kept inline so agentmux-common doesn't have to depend on agentmux-launcher" - launcher depends on common, so launcher can call `agentmux_common::runtime_mode::fnv1a_64` (8 lines + its test) |
| `suppress_os_crash_dialogs` | `cef/src/lib.rs:70`, `launcher/src/main.rs:94` | 15 lines each + non-Windows stub; move to `common::win32` |
| `version_core` | `launcher/src/upgrade_restart.rs:68`, `srv/src/bootstrap/carry_forward.rs:29` | 6 lines |
| `valid_session_id` | `remote/src/daemon.rs:68`, `srv/src/backend/blockcontroller/durable_ssh.rs:131` | comment in srv says it mirrors remote's; srv already depends on `agentmux-remote` -> import it |
| `extract_commented_setting_key` | `cef/src/commands/platform.rs:1206`, `srv/src/backend/wconfig/loader.rs:190` | 8 lines |
| `bind_first_pipe_instance` | `launcher/src/ipc/server.rs:105`, `srv/src/srv_ipc/server.rs:45` | |
| `config_dir` | `launcher/src/background_config.rs:136`, `launcher/src/splash_config.rs:75` | same crate |
| `merge_known` | `srv/src/registry/def_store.rs:251`, `registry/store.rs:516` | 15 lines |
| `marker_version` | `srv/src/backend/transcript_backfill.rs:305`, `registry/def_migrate.rs:200` | |
| `acp`/`msp` `ensure_started`, `send_message`, `publish_status` | `blockcontroller/acp.rs:908/919/326`, `msp.rs:188/198`, `subprocess/mod.rs:302` | 6-7 lines each |
| `to_ipv4` v6 handling | `srv/.../lan_discovery.rs:204`, `drone/executor/blocks/api.rs:139` | both under `allow(deprecated)` |
| `UIContext` struct defined twice | `srv/src/backend/obj.rs:189`, `srv/src/backend/service.rs:37` | |
Total duplicated code is small (~150 lines); value is consistency, not size.

---------------------------------------------------------------------------
## 4. Leftovers from recent removals

### Vacuum row / `run_saga_vacuum`
- `run_saga_vacuum`, `runSagaVacuum`, `saga-vacuum`, `upgrade:saga-vacuum-done`, `rows_deleted`: **no occurrence in crates/, frontend/, scripts/, schema/, sdk/, test/, .github/, Taskfile, settings-template**. Only in the old spec `docs/specs/SPEC_UPGRADE_PANEL_2026_06_27.md:24,27,413-414,431,467,475,488-489,522,590,592` (docs).
- `frontend` has no "vacuum" string (only unrelated `public/fontawesome/css/pro-icon-aliases.css:318 fa-vacuum-robot`). No RPC stub/event/config key remains.
- Remaining Rust remnants (all comments or test-only vocabulary, the launcher saga log is now in-memory per `docs/specs/SPEC_PILLAR1_STEP6_SAGA_COLLAPSE_2026_07_16.md`):
| file:line | remnant | conf | action |
|---|---|---|---|
| `launcher/Cargo.toml:41-44` | chrono `serde` feature justified by "the vacuum cutoff parameter" | LIKELY stale | see D1 |
| `launcher/src/saga/mod.rs:87-88` | "LSD-1 (PR LSD-1) - durable launcher saga log + API. Foundations only: the coordinator does NOT call any of these methods yet." directly above the note that says the opposite | CONFIRMED stale comment | delete |
| `launcher/src/saga/mod.rs:191,231,256,265-267,306-307,390,435,459-461,629,649-652,768,802,1000,1055,1633,1664,1794-1797` | many `LSD-2`/`LSD-3` doc comments describing the durable log / "walker will upgrade it to failed_compensation" / "persisted to `<data-dir>/db/launcher-sagas.db`" | CONFIRMED stale | rewrite/trim |
| `launcher/src/supervisor/unix.rs:37,798`, `supervisor/windows.rs:372,1297` | "launcher-sagas.db", "LSD-3 compensation pass", "durable saga log is now installed" | CONFIRMED stale | trim |
| `launcher/src/main.rs:369` | "`--diag sagas` is OFFLINE: it reads the ..." | CONFIRMED stale | trim |
| `launcher/src/diag.rs:694-710` | doc block says the diag opens the SQLite log read-only via `open_read_only` + `snapshot_recent(50)`; the impl (`diag.rs:727-747`) now only prints "durable saga log was removed". Also the duplicated `run_sagas_diag` (`:711` cfg windows / `:716` cfg not windows) wrappers are identical. `#[cfg(test)] mod sagas_diag_tests` (`:750`+, "run the recovery walker manually" at :811) tests a formatter that no longer exists (tests the in-memory log only) | CONFIRMED stale | collapse to one fn; drop or retarget test; keep `--diag sagas` stub only if users still type it |
| `launcher/src/saga/log/mod.rs:49,83-84,367` and `diag.rs:812-838`, `saga/log/tests.rs:113-153` | `SagaOutcome::FailedCompensation` + `mark_failed_compensation` kept "for API parity and tests" (`#[allow(dead_code)]` at :83); sole production caller (the walker) is gone; only tests call it | CONFIRMED test-only | delete variant/fn + 4 tests (about 30 lines) |
| `launcher/src/saga/log/mod.rs:155-200` `StepRecord`/`SagaRecord` `#[cfg_attr(not(test), allow(dead_code))]` | fields kept only for tests | LIKELY |  |
| `launcher/src/supervisor/{windows.rs:331-339,unix.rs:258-262}` | best-effort deletion of legacy `launcher-sagas.db` | intentional cleanup | keep for a few releases, then drop |
| `srv` `sagas.db` (srv's own saga log) | live (`bootstrap/stores.rs:522`) - not a remnant | - | - |

### Windows update path that used tar.exe
- `rg 'tar\.exe|System32.*tar'` over crates/: no code path invokes `tar.exe` any more. Windows uses `update_unpack::unpack_7z` (`sevenz-rust2`, `update_download.rs:530-547`).
- Remnants: (a) `srv/src/backend/update_download.rs:506-509` `fn tar_exe() -> PathBuf { PathBuf::from("tar") }` - a trivial wrapper left from when it returned the System32 path; used at :549 and test :799. Also referenced by name in the process-spawn audit table `srv/src/backend/pane_env.rs:443` (`("src/backend/update_download.rs", "tar_exe(", 2, ...)`), so inline-or-keep must update that table (CONFIRMED vestigial, trivial). (b) `update_download.rs:546-551` the non-Windows branch still does `use agentmux_common::win32::NoWindow` + `.no_window()` (no-op off Windows). (c) comments `update_download.rs:25,529,722`, `update_unpack.rs:9,114` are accurate explanations, not remnants. (d) `update_download.rs:443` doc "Where in what `tar` unpacked" is only half-true now (Windows is 7z).
- `Cargo.toml` `tar = "0.4"` in srv is still used (`container.rs:802,810`, `tool_store.rs:504`) - not a leftover. `zip` (239 hits) and `flate2` still used.

---------------------------------------------------------------------------
## 5. Estimated savings and priority
| rank | item | lines | risk |
|---|---|---|---|
| 1 | F2 ijson stack (3 files + tests + 2 option fields) | ~590 prod + ~400 test | low-med (regenerate ts-rs types) |
| 2 | F1 `userinput.rs` | 203 prod + 180 test | low |
| 3 | 2a/2b/F4 allow-hidden dead + test-only items in srv (`base`, `shellexec`, `service`, `mps`, `blockcontroller/mod`, `reactive`, `rpc/engine`, `storage/*`, `eventbus`, `wcore/event`, messaging) | ~570 no-ref + ~1390 test-only (overlaps ranks 1-2 by ~300) | low; items documented as "unwired until spec X" should be left |
| 4 | F3 rpc_types Go leftovers | ~190 (+ srv-types.d.ts counterpart) | low |
| 5 | Remove 13 crate-wide `#![allow(dead_code)]` after cleanup so the compiler guards the rest | 13 attrs | none |
| 6 | Launcher saga-collapse leftovers (comments, diag wrappers, FailedCompensation) | ~120 lines | none |
| 7 | F6 duplicates (fnv1a_64, valid_session_id, suppress_os_crash_dialogs, version_core) | ~100 lines | low |
| 8 | D3/D4 dependency hygiene, version unification | 2 manifest lines; axum/which/png unification could remove several crates (not measured) | axum migration medium |
Total realistic removable: ~2.0-2.4k production lines and ~1.0k test lines (about 1% of the Rust code base, 2-3% of srv prod code). Binary size effect ~0 (already linker-stripped); compile time gain small. Zero removable dependencies.

## 6. What was NOT checked / limits
- No compilation, so `#[allow]` removals and stale-allow claims in the "referenced elsewhere" class (63 srv, 13 cef) are unverified; name collisions can hide dead items (conservative direction) - those items are not in any list above.
- Enum variants and struct fields under allow (46 srv, 8 cef, 9 launcher; mostly serde wire shapes in `messaging/{slack,telegram,discord}/types.rs`, `rpc/engine.rs`, `storage/filestore/cache.rs`) were listed, not individually judged.
- Per-platform deadness: an item used only from `cfg(windows)` code looks live in a name index even though it is dead on Linux; items only referenced from cfg-dead branches were not separated.
- cef `wrap_*!` macro bodies were treated as live roots; CEF callback fns are not individually audited.
- RPC command names registered by string (`register_typed("fs.trash", ...)`) were not cross-checked against frontend/MCP callers (the dead-RPC-command question); `scripts/gen-rpc-stubs.mjs` / `gen-mux-events.mjs` outputs not diffed against handlers.
- Docs/specs mentions (`docs/**`) were excluded from reference counts on purpose.
