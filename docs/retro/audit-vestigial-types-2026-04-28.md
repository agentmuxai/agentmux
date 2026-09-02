# Audit: Vestigial WaveTerm/WaveMux Types in AgentMux

**Date:** 2026-04-28
**Scope:** Pre-Phase-E cleanup audit. AgentMux was rebranded from WaveTerm; this surveys types, names, and abstractions that still mirror the WaveTerm era.
**Method:** Static grep + read across `agentmux-srv`, `agentmux-cef`, `agentmux-launcher`, `agentmux-common`, `frontend/`, and `specs/`. No runtime tracing.
**Output:** Triage table — one row per candidate.

---

## Summary

- **Total candidates examined:** 23
- **Recommended for delete:** 4 (3 dead env vars / config + 1 unused field set)
- **Recommended for rename:** 9 (`Wave*` symbol-set rebrands)
- **Recommended for merge into another type:** 1 (`Workspace.{icon,color}` into `Window.meta`; `Workspace` itself stays — see row)
- **Recommended for archive (specs):** 12 (shipped or stale Drafts older than ~6 weeks)
- **Recommended keep-as-is:** 9 (Wave-named modules with real abstractions; rename later if name still rankles)

### Top 3 highest-value cleanups (blast-radius vs payoff)

1. **Drop `WCLOUD_ENDPOINT` / `WCLOUD_WS_ENDPOINT` env vars.** Set by both launcher and cef, never read by srv (zero `WCLOUD` reference in `agentmux-srv/src/`). Pure superstition. Deleting the two `.env(...)` calls is safe and removes the only `WCLOUD_*` surface still active in code. Blast radius: tiny (4 lines across 2 files).
2. **Rename `WAVESRV-ESTART` / `WAVESRV-EVENT:` wire markers to `AGENTMUXSRV-ESTART` / `AGENTMUXSRV-EVENT:`.** The hardcoded string is one of the last user-visible WaveTerm names — it appears in srv stderr at every startup and in launcher logs. Producers: srv `main.rs:516`, srv `tests/integration_test.rs:27`. Consumers: launcher `srv_spawner.rs:210,226`, cef `sidecar.rs:303,314,474`. Coordinated change in 4 files; symmetric on both sides; small comment churn. Phase E is touching srv-spawn anyway.
3. **Rename TS `WaveWindow` to `Window` (or `Win`) and `WaveObj`/`WaveObjUpdate`/`waveWindow` accordingly in `frontend/types/gotypes.d.ts` and 30 frontend files.** The Rust side already uses `Window` (not `WaveWindow`); the TS naming is the inconsistency. Pure rename, no semantics change. Blast radius: medium (~30 frontend files), but tractable as a single PR with `rg -l` + `replace_all`.

Note: the original guess that `Workspace` could fold into `Window` does **not** survive close inspection. Tear-off (block + tab) creates new workspaces independently of windows (`wcore::dnd::tear_off_block`, `tear_off_tab`), and a workspace is the natural cascade root for tab/block deletion (`close_window` deletes the workspace, but the workspace also exists transiently before its window does in tear-off paths). 1:1-with-window is the *steady state* but not invariant. See row 1 below.

---

## Triage Table

| Type / Name | Defined at | Referenced in | Still earns its keep? | Recommended action |
|---|---|---|---|---|
| `Workspace` struct (Rust) | `agentmux-srv/src/backend/obj.rs:346` | `wcore/{workspace,window,tab,dnd,mod}.rs`, `server/service.rs` (~89 hits), `storage/wstore.rs`, `frontend/types/gotypes.d.ts:1624` (~30 frontend files via `Workspace` & `WorkspaceListEntry`) | **Yes, conditionally.** Workspace owns `tabids`, `pinnedtabids`, `activetabid`. These are real fields with no sensible home on `Window`. Tear-off creates fresh workspaces, so the relationship is logically 1:N-from-window-over-time even if 1:1 in steady state. Folding into Window would conflate "what's on the screen" with "what kind of screen." | **keep-as-is** (small). Revisit only if Phase E moves tab-ownership off Workspace. The `name`/`icon`/`color` fields, however, see next row. |
| `Workspace.icon` / `Workspace.color` fields | `agentmux-srv/src/backend/obj.rs:352-354` | Backend: `wcore/workspace.rs` (passed through `create_workspace`), `WORKSPACE_COLORS`/`WORKSPACE_ICONS` constants in `wcore/mod.rs:34-53`. Frontend: zero usages of `workspace.icon` or `workspace.color` (only stub stub `WorkspaceService.GetColors`/`GetIcons` in `services.ts:523-526`, never called). | **No.** Vestigial WaveTerm "named workspace switcher" UI that AgentMux never reimplemented. `WORKSPACE_ICONS[0]` is `"custom@wave-logo-solid"` — a literal Wave logo reference. | **delete** (small): drop fields, the two color/icon constants, and the dead `GetColors`/`GetIcons` service stubs. Keep `Workspace.name` (used by `InstancePanel.tsx:120` as a window display fallback). |
| `Workspace.name` field | `agentmux-srv/src/backend/obj.rs:350` | `InstancePanel.tsx:120` (fallback window display name); set via `UpdateWorkspace` service (which has zero callers). | **Marginally.** The only read site uses it as a *fallback* after `window:displayname`. The user has no UI to set it. So it's really just always `""`. | **delete** if Phase E moves the display-name story onto Window meta directly. **keep-as-is** if you want the fallback path for legacy DBs (small blast). |
| `wcore` module | `agentmux-srv/src/backend/wcore/mod.rs` | All of `agentmux-srv`; one of the central modules (workspace.rs, window.rs, tab.rs, block.rs, dnd.rs, event.rs, mod.rs ~676 lines). | **Yes — the code earns its keep, but the *name* is pure rebrand candidate.** "Wave Core" comment block at line 5 still says "Wave Core: application coordinator." It really is the workspace/window/tab/block CRUD facade. | **rename** to `core` or `entities` (medium). Touches every backend file that does `use crate::backend::wcore` — about a dozen call sites. Mechanical. |
| `wconfig` module | `agentmux-srv/src/backend/wconfig/mod.rs:5` ("Port of Go's pkg/wconfig/.") | `backend/{config_watcher_fs,blockcontroller/shell,rpc/router,rpc_types,reactive,...}.rs` — 29 hits across 10 files. | **Yes; rename-only candidate.** It's the settings loader; nothing wave-specific. | **rename** to `config` (medium). Conflicts with existing `crate::config` (the CLI-args `Config`) — pick `appconfig` or `usrconfig` to disambiguate. |
| `wps` module (Wave Pub/Sub) | `agentmux-srv/src/backend/wps.rs:5` | Internally imported as `wps::{EVENT_*, Broker}` across blockcontroller, server, wcore; ~15 files. | **Yes; rename-only.** Just a generic pub/sub broker. | **rename** to `pubsub` or `eventbroker` (medium — affects `EVENT_WAVE_OBJ_UPDATE` constant name too, see below). |
| `wshutil` module | `agentmux-srv/src/backend/wshutil/mod.rs` | Used by `backend/blockcontroller`, `backend/rpc`, `agentmux-srv/src/server`. ~30 hits. | **Partly.** "WSH" was Wave Shell; `agentmux-wsh` crate has been retired (per `SPEC_RETIRE_WSH_2026_04_12.md`). The remaining `wshutil` is OSC encoding, RPC proxy, event listener, I/O adapters — used by the agent and terminal panes. The *name* dangles. | **rename** to `rpcutil` or `osc_rpc` (medium). |
| `WshRpc` (in `wshutil/wshrpc.rs:144`) | `wshrpc.rs` | `wshutil/mod.rs`, `rpc/engine.rs:188` (which already says "Port of Go's `WshRpc`" and renamed itself to `WshRpcEngine`) | **Yes; the `WshRpc` struct in `wshrpc.rs` is the original transport-only RPC client; `WshRpcEngine` is the dispatch engine. Both are used.** Names are wave-flavored but not duplicated logic. | **rename** in lockstep with `wshutil` rename (medium). `WshRpc` → `OscRpc`, `WshRpcEngine` → `RpcEngine`. |
| `WaveStore` (SQLite wrapper) | `agentmux-srv/src/backend/storage/store.rs:23` | 39 hits internal + 9 across server/blockcontroller/wcore call sites; 2070 lines. | **It's a real abstraction (generic `WaveObj`-typed CRUD over SQLite + a transaction wrapper), but it's a "thin trait around SQLite + serde_json"**. The genericity is justified by the OType dispatch. | **rename** to `ObjStore` (medium). Logic stays. Touches the `WaveStore::` and `&WaveStore` references — about 70 sites; mechanical. |
| `WaveObj` trait | `agentmux-srv/src/backend/obj.rs:121` | 17 hits in `obj.rs` + impl macros for Client/Window/Workspace/Tab/Block/LayoutState; used by `WaveStore` generics. | **Yes — this is the polymorphic store abstraction (otype-tagged objects with a uniform CRUD shape). It does unify 6 distinct types across a single dispatch path. Not vestigial.** Name is wave-flavored. | **rename** to `Obj` (or `Entity`) (medium). Lockstep with WaveStore rename. ~17 trait-method implementations + use sites in `wstore.rs` generic bounds. |
| `WaveObjUpdate` struct (Rust) + TS `WaveObjUpdate` | `agentmux-srv/src/backend/obj.rs:462`; `frontend/types/gotypes.d.ts:1536` | Rust: returned in WS update payloads. TS: only in `gotypes.d.ts` as a generic update envelope. | **Yes.** It's the websocket update envelope (`{updatetype, otype, oid, obj}`) — a real schema. | **rename** to `ObjUpdate` (medium). Wire format `{updatetype, otype, oid, obj}` is shape-not-name, so safe across boundaries if both sides updated together. |
| `wave_obj_to_value` / `wave_obj_to_json` / `wave_obj_from_json` helpers | `agentmux-srv/src/backend/obj.rs:478,488,498` | 63 call sites across 8 files (server/service.rs:26, server/app_api.rs:6, blockcontroller/{persistent,subprocess,shell}.rs, wstore.rs, websocket.rs). Most heavily in service.rs (every Update emits `wave_obj_to_value(&...)`). | **Yes — actually does dispatch over multiple types via the `WaveObj` trait. `wave_obj_to_value` injects the runtime `otype` field that the wire requires for polymorphic updates. This isn't one-type-per-call.** | **rename** to `obj_to_value`, `obj_to_json`, `obj_from_json` (medium). Tied to the `WaveObj` → `Obj` rename above. |
| TS `WaveObj` / `WaveWindow` aliases | `frontend/types/gotypes.d.ts:1528,1564` | ~30 frontend files (`app-init.ts`, `store/global.ts`, `store/services.ts`, `wos.ts`, `InstancePanel.tsx`, all view models, all layout files…). | **Yes — same role as Rust trait, but on the TS side `WaveWindow` is *just* `Window`. There's a name asymmetry: backend Rust calls it `Window`; TS calls it `WaveWindow`.** | **rename** TS `WaveWindow` → `Window` (or alias `WaveWindow = Window` for one release then drop). `WaveObj` → `Obj`. Medium blast (~30 files), all mechanical. |
| `WAVESRV-ESTART` / `WAVESRV-EVENT:` wire markers | Producer: `agentmux-srv/src/main.rs:516`. | Consumers: `agentmux-launcher/src/srv_spawner.rs:210,226,314`; `agentmux-cef/src/sidecar.rs:303,314,474`; `agentmux-srv/tests/integration_test.rs:27`. Six total sites. | **No.** This is a literal-string handshake between processes. Renaming it is a coordinated 6-line change inside this repo (no external consumers). It's the most user-visible WaveTerm artifact in the runtime — leaks into stderr on every srv startup. | **rename** to `AGENTMUXSRV-ESTART` / `AGENTMUXSRV-EVENT:` (small, but coordinated). Highest payoff per LOC. |
| `WCLOUD_ENDPOINT` / `WCLOUD_WS_ENDPOINT` env vars | Set in `agentmux-launcher/src/srv_spawner.rs:128-129` and `agentmux-cef/src/sidecar.rs:230-231`. | **Zero readers** in `agentmux-srv/src/`. Confirmed via `rg "WCLOUD" agentmux-srv/src` → 0 hits. | **No.** They point at `api.agentmux.ai` but nothing in the running process reads them. Pure ritual. | **delete** (small — 4 lines across 2 files). |
| `--wavedata` CLI flag (srv) | Producer: `agentmux-launcher/src/srv_spawner.rs:117`, `agentmux-cef/src/sidecar.rs:207`. Consumer: `agentmux-srv/src/config.rs:8` (Clap `arg(long = "wavedata")`). | 3 source sites + 4 test/doc references. | **It's an active flag**, but the name is a 1:1 WaveTerm rebrand candidate. | **rename** to `--data-dir` (small). Update the two callers in lockstep with the Clap definition. |
| `WAVETERM_DEV` / `WAVETERM_DEV_VITE` env vars (frontend) | `frontend/util/isdev.ts:7-8` (`WaveDevVarName`, `WaveDevViteVarName`). | Read via `getEnv()` for `isDev()`/`isDevVite()` flags. | **Active code path, but the literal env var name is unmigrated.** Memory says elsewhere `WAVETERM_*` → `AGENTMUX_*` was already done; these two slipped through. | **rename** env vars to `AGENTMUX_DEV` / `AGENTMUX_DEV_VITE` and rename the consts to `AgentMuxDevVarName` etc. (small — single file change + whatever sets them in build env). |
| `WAVETERM_AUTH_KEY` (test only) | `agentmux-srv/tests/integration_test.rs:12` | Test code only. | **No.** Production uses `AGENTMUX_AUTH_KEY` (`config.rs:34`); this is a stale test fixture. Test will not actually authenticate srv with this name today. | **delete / fix** (small). Either the test is broken (unlikely — would have been noticed) or it's redundant. Either way, replace with `AGENTMUX_AUTH_KEY`. |
| `EVENT_WAVE_OBJ_UPDATE`, `EVENT_WAVE_AI_RATE_LIMIT`, `EVENT_WORKSPACE_UPDATE` event-type constants | `agentmux-srv/src/backend/wps.rs:27,42,44` | `EVENT_WAVE_OBJ_UPDATE` widely used (waveobj:update wire string). `EVENT_WAVE_AI_RATE_LIMIT` flagged `#[allow(dead_code)]`. `EVENT_WORKSPACE_UPDATE` flagged `#[allow(dead_code)]`. | **`EVENT_WAVE_OBJ_UPDATE` earns its keep** (wire string `"waveobj:update"` is consumed by every frontend subscription). The other two are dead — `dead_code` allow attributes are a tell. | `EVENT_WAVE_OBJ_UPDATE`: **rename const to `EVENT_OBJ_UPDATE`** but keep wire string as `"waveobj:update"` for now (or migrate both — but wire compat with existing dev DBs may bite). The other two: **delete**. (small) |
| `COMMAND_STREAM_WAVE_AI`, `COMMAND_AI_ENABLE_TELEMETRY`, `COMMAND_GET_AI_CHAT`, `COMMAND_GET_AI_RATE_LIMIT`, `COMMAND_AI_TOOL_APPROVE`, `COMMAND_AI_ADD_CONTEXT` | `agentmux-srv/src/backend/rpc_types.rs:198,256-260` | Only `COMMAND_GET_AI_RATE_LIMIT` is registered (`websocket.rs:589`) — and the handler returns a hardcoded "no rate limits" stub. The other 5 constants have zero references outside their declaration. **No frontend code calls any of them.** | **No.** All 6 are leftovers from WaveTerm's WaveAI panel feature, which `docs/specs/archive/remove-aipanel-sidebar.md` says was deleted. | **delete** the five unused constants. **delete or simplify** `COMMAND_GET_AI_RATE_LIMIT` and its stub registration — the frontend never calls it. (small) |
| `get_wave_init_opts` IPC command | `agentmux-cef/src/commands/backend.rs:26`; dispatch in `ipc.rs:210`. | No frontend caller (renamed to `onAgentMuxInit` callback path; this stale Rust-side name lingers). | **Yes, the function itself feeds `onAgentMuxInit`**, but the name is wave-flavored. | **rename** to `get_init_opts` (small — 2 sites in cef + the IPC string). |
| TS `WaveObjUpdate.obj?: WaveObj` polymorphic shape | `frontend/types/gotypes.d.ts:1540` | Used wherever `WebReturnType.updates: WaveObjUpdate[]` is consumed. | **Yes** — same role as Rust side. | Rename in lockstep with TS `WaveObj` → `Obj` (medium). |
| `meta_get_string`, `meta_get_bool`, `MetaMapType`, `merge_meta` | `agentmux-srv/src/backend/obj.rs:59,65,105,113` | Used everywhere object meta is touched. | **Yes — generic meta-map helpers.** Not WaveTerm-specific despite living in `obj.rs`. | **keep-as-is** (no action). |
| `ORef` (otype:oid string) | `agentmux-srv/src/backend/oref.rs:17` | Wire-format identifier; passed everywhere. | **Yes — concrete abstraction tied to the polymorphic store; not vestigial.** | **keep-as-is**. |

---

## Specs Triage (`specs/` — 87 files)

Method: read header (Status / Date / explicit "Implemented") and cross-check key features against current code (`frontend/`, `agentmux-srv/`).

### Recommended for `specs/archive/` (shipped or executed)

| Spec | Reason |
|---|---|
| `SPEC_PANE_CYCLE_FOCUS.md` | Header says `Status: Implemented` (2026-03-05). |
| `widget-icon-only.md` | Header says `Status: Implemented`, PR #69 (2026-03-07). |
| `SPEC_RETIRE_WSH_2026_04_12.md` | `agentmux-wsh` crate no longer exists — spec was executed. |
| `per-pane-zoom.md` | Per memory: merged at v0.31.90 (PR #86). |
| `pane-context-menu.md` | Per memory: PR #360 fixed context-menu event name; v1 spec body acknowledges shipped. |
| `SPEC_TAB_DND_REORDER.md` | `frontend/app/tab/{tabbar,tab,droppable-tab}.tsx` all have `dragstart`/`draggable` paths; shipped. |
| `SPEC_TAB_DND_DIAGNOSIS.md` | Sibling diagnosis doc — companion to the shipped reorder spec. |
| `SPEC_TAB_DND_ANIMATION.md` | Same. |
| `solidjs-appimage-launch-fix.md` | `Status: Required` (per-fix); AppImage builds active in Taskfile. |
| `chrome-zoom-linux-label-shift.md` | `Status: Fixed`. |
| `tabbar-enhancements.md` | Dated 2026-02-12 (oldest open Draft); all three sub-features either shipped or abandoned (`netstat -ano | grep` for "tabbar version" finds nothing in `frontend/`). |
| `bench-results-20260329.md` | Historical bench output — not active design surface. |

### Likely-stale Drafts to flag for owner triage (no clear shipped/abandoned signal)

These are old-ish (>3 weeks old as of 2026-04-28) and labeled Draft. They may still matter; flagging only:

`tauri-vs-cef-performance.md`, `swarm-analysis.md`, `tab-styling.md`, `themes-spec.md`, `web-widget.md`, `widget-dnd-reorder.md`, `cef-isolation-audit.md`, `cef-portable-layout.md`, `lan-awareness-and-embedded-jekt-api.md`, `local-messagebus-architecture.md`, `process-lifecycle-v2.md`, `console-flash-report.md`.

### Specs that look active (keep where they are)

Anything dated 2026-04-13 or later AND labeled Draft that I didn't tag above (most `SPEC_AGENT_*`, `SPEC_FORGE_*`, `SPEC_TOOL_OVERLAY_*`, `SPEC_SLASH_*`, `SPEC_BROWSER_PANE_*`, `SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md`, `ANALYSIS_*_2026_04_27.md`).

`SPEC_REMOVE_WEBVIEW.md` is dated 2026-03-05 / `Status: Pre-implementation` — but `frontend/app/view/browser/` still has the full browser pane. Either the spec was rejected or never executed; the owner should decide.

---

## Notes on `Workspace` 1:1 question

Asked: "is the 1:1 strict in all code paths?" Looking at the actual graph:

- `wcore::create_window` / `create_window_full` always associate the new window with a workspace (creating one if none provided).
- `wcore::tear_off_block` / `tear_off_tab` (`dnd.rs:195,406`) create a brand-new workspace **before** the new window exists — it's a transient orphan workspace until the frontend opens a window for it (see `frontend/app/drag/CrossWindowDragMonitor.*.tsx:223,240` calling `WorkspaceService.TearOffBlock`/`TearOffTab` and only then opening a window for the returned workspace ID).
- `service.rs` `("workspace", "DeleteWorkspace")` deletes by id without a window check.
- `Window.workspaceid` is a single-value string; **a window cannot point at multiple workspaces**.

**Conclusion:** AgentMux preserves WaveTerm's "workspace = container of tabs that a window can point at" model. It is *not* a 1:1 rebrand candidate at the type level — but the workspace's *user-facing fields* (`name`, `icon`, `color`) are vestigial because no UI surfaces them. Hence the split recommendation: keep the type, drop the dead fields.
