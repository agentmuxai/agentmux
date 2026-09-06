# DRY and modularity audit — findings and a slimming plan

**Status:** proposed
**Date:** 2026-09-06
**Author:** Manoz@Area54
**Baseline:** `main` @ `734214c7f` (v0.55.37)

**Ask (repo owner):** *"a rigorous, deep analysis of the code base for DRYness and clean modularity. Identify clear places where we can slim things down with DRY and, as an extension, how that may play into better modularization of the architecture."*

---

## 0. The short version

The codebase is **not** riddled with copy-paste — measured duplication is 2.8% (frontend) and 3.2% (Rust), and frontend utility helpers are genuinely centralized. What it has instead is **structural** duplication with a small number of root causes, and the code knows it: **334 "keep in sync" / "mirror" comments across 210 distinct source files** — about one file in six — each marking a place where the same knowledge lives twice and a human is the sync mechanism.

Five root causes account for almost all of it. In payoff order:

| # | Root cause | Evidence in one line | Fix shape |
|---|---|---|---|
| 1 | **No codegen across the Rust↔TypeScript boundary** | 322 hand-maintained RPC stubs + a 2,819-line hand-maintained `gotypes.d.ts`; no generator script exists | Generate bindings + types from srv's `rpc_types` |
| 2 | **`agentmux-common` isn't used as a common** | 8K lines shared vs 300K Rust; `CREATE_NO_WINDOW` in 21 files (private *twice* inside common itself); `now_ms` ×20 (absent from common); `event_log.rs` copied whole between two crates | Grow common into a real shared layer; dep graph is a clean star, so zero cycle risk |
| 3 | **Whole-file platform forks** | `.platform` resolver swaps entire files; `zoom.{win32,linux,darwin}.ts` differ by **9 comment lines** out of 186 | Shared core + thin per-platform override |
| 4 | **Twin primitives built by copy-rename** | `mcp` ↔ `skill` are 58–71% structurally identical across all four layers | A "managed primitive" abstraction |
| 5 | **Parallel frameworks sharing only vocabulary** | saga + reducer exist in both launcher and srv; the two saga `mod.rs` share exactly one method name (`new`) | A decision, not a refactor: share it or document the split |

Everything else is contained — a handful of god files, two same-file render-tree duplicates, and a fragmented process-kill — and is listed in §3.

## 1. Method

- **Sizes** — line counts per crate/file via `find`+`wc`; top-N largest files.
- **Duplication** — `jscpd` 4.x, `--min-tokens 60 --min-lines 8`, tests excluded, run separately over `frontend/` (ts/tsx) and the six Rust crates. Token-based, so it catches renamed copies that `diff` misses (this mattered — see §2.4).
- **Verification of every clone cited** — each headline pair was re-checked with `diff` (line-level) and, for renamed twins, with identifier-normalized `diff`, so the similarity numbers below are two independent measures agreeing, not one tool's output.
- **Self-annotation** — grep for `kept in sync | must match | mirror of | by convention, not shared | copied from` in non-test sources. This is the codebase's own confession list and turned out to be the single most useful signal.
- **Symbol collisions** — `fn`/`const` names defined in ≥2 non-test files; `pub struct`/`enum` names defined in >1 crate.
- **Dependency graph** — path deps from each `Cargo.toml`.
- **Excluded on inspection** (measured, then rejected as *not* duplication): `host_spawn.rs` vs `container_spawn.rs` (1239/1485 lines differ), `useWindowDrag.win32` vs `.linux` (325/409 differ), trait-impl fan-out like the 25× `fn up / scope / description` (one per migration — that's polymorphism, not copying).

Everything measured is reproducible from a clean checkout; the two `jscpd` JSON reports are the only artifacts and take ~20 s each to regenerate.

## 2. Findings — structural (the five root causes)

### 2.1 The Rust↔TypeScript boundary is hand-synced end to end

| Surface | Count | Generated? |
|---|---|---|
| `frontend/app/store/rpc-api/*.ts` command stubs | **284** | No — header: *"Hand-maintained RPC bindings. Keep in sync with the agentmux-srv RPC…"* |
| `frontend/app/store/services.ts` service methods | **38** | No — same header |
| `frontend/types/gotypes.d.ts` | **2,819 lines** | No generated marker; the name is a Go-era leftover |
| srv `rpc_types` `Command*` structs | 110 | — the source these should derive from |
| Generator script in `scripts/` | **none** | — |

The consequence is spread, not concentrated: the 334 sync comments cluster at **max 8 per file**. Representative confessions:

- `agentmux-srv/src/backend/blockcontroller/persistent.rs:785` — *"KEEP IN SYNC (no shared constant crosses the Rust/TypeScript boundary for…"*; its twin `frontend/app/view/agent/hooks/useAgentQuestions.ts:86` says the same words.
- `frontend/app/tab/tab-presets.ts:45` — the default layout tree, *"a SEPARATE mechanism kept in sync by convention, not shared code"* with `wcore::default_three_pane_tree`. I hit this one directly this week (PR #2988): the frontend preset was still four panes after the backend went to three, and its own comment claimed sync. It also **cannot express the backend's 20/80 sizing** — the applier only emits even splits — so the two are not even capable of agreeing.
- Provider catalog: `agentmux-srv/src/backend/providers.rs` (1,206 lines), `agentmux-cef/src/commands/providers.rs` (822), `frontend/app/view/agent/providers/catalog.ts` (573) — **2,601 lines across three crates and two languages** defining the same registry, with `pinned_version` tracked by hand in all three (`providers.rs:211`, `catalog.ts:118`, cef `providers.rs:200` each say "Keep in sync with…").
- Cross-language **formatter mirrors** — TypeScript re-implementations of Rust functions so a preview matches the file the backend will write: `frontend/app/view/brain/global-brain-model.ts:36` (*"Mirror of the backend format_global_brain_block"*) and `frontend/app/view/agent/agent-color.ts:63` (*"Mirrors agent_color.rs::dim_agent_color"*).

**Why it's root cause #1:** every other boundary finding (layout tree, provider catalog, formatter mirrors, the 322 stubs) is a symptom of the same missing tool. Codegen from `rpc_types` + a shared-constants module removes a whole *class* of sync comment rather than one instance.

### 2.2 `agentmux-common` exists but isn't used as a common

`agentmux-common` is **12 files / 8,202 lines** against ~300K lines of Rust — and the workspace dependency graph is a clean star (`srv`, `cef`, `launcher`, `bashwrap`, `mcp` → `common`; `common` → nothing). **There is no cycle risk in lifting anything into it.** Yet:

| Duplicated symbol | Where | What common has |
|---|---|---|
| `const CREATE_NO_WINDOW` | **21 files across 5 crates** (srv ×15) | Declared **privately, inside two functions** — `cli.rs:37`, `runtime_mode.rs:431`. Common holds it twice and exports it to no one. |
| `fn now_ms` / `fn now_secs` | **20 + 6 files**, all the same 3-line `SystemTime::now().duration_since(UNIX_EPOCH)` body (i64/u64 and `unwrap_or_default`/`map` variants) | Nothing |
| `event_log.rs` | `agentmux-srv/src/event_log.rs` and `agentmux-launcher/src/event_log.rs` — **415 lines each, 56 lines differ, every one a comment or the log filename**. The srv copy's own header: *"Mirror of agentmux-launcher's event log… the in-memory ring + replay semantics are identical."* | Nothing |
| ObjC FFI externs (`objc_msgSend`, `sel_registerName`, `objc_getClass`, `method_setImplementation`) | Re-declared per file in **9 `agentmux-cef` files** (~55 `extern "C"` declarations) | Nothing |
| Process-kill | **Five implementations, three mechanisms**: `TerminateJobObject` (`process_tracker/windows.rs`), `taskkill /F /T /PID` in four separate crates (`bashwrap/bash_wrap.rs:419`, `cef/commands/cli_login.rs:1454`, `srv/backend/shell_node.rs:299`, `srv/identity/auth_session.rs:391`), `libc::kill` in three more | Nothing |
| Broadcast chunking constants | `agentmux-mcp/src/main.rs:2486` — *"Mirrors `fleet_broadcast_impl`'s own chunking constants — kept in sync by hand since this is a separate process/crate"* | Nothing — and `agentmux-mcp` **already depends on common** |
| Cross-crate types | `WindowKind`, `Rect` in cef *and* common; `ProviderConfig` in cef *and* srv; `DataPaths` in common *and* launcher; `Event`/`Command` in common *and* srv | Partially — the common versions exist and are shadowed |

(All four `taskkill` sites correctly use `/PID`, per CLAUDE.md's ban on image-name kills. The finding is fragmentation, not a safety bug.)

**Why it's root cause #2:** these are the cheapest wins in the audit — each is a mechanical lift with a passing test suite behind it — and they compound: once `common::process`, `common::time`, `common::win32` exist, the *next* duplicate has somewhere obvious to go.

### 2.3 Platform forks are whole-file, even when nothing differs

`vite.config.ts:50` `platformResolve` rewrites `import "x.platform"` → `x.<win32|linux|darwin>.ts`. It resolves a **file**, so every platform variant must be complete — there is no shared-core-plus-override shape. Five module families use it:

| Family | Sizes (win32/linux/darwin) | Measured divergence | Verdict |
|---|---|---|---|
| `app/store/zoom.*.ts` | 186 / 185 / 187 | **9 lines differ, all comments** (`diff` verified) | Three identical copies |
| `layout/lib/TileLayout.*.tsx` | 597 / 535 / 520 | 256 (win↔linux), 149 (linux↔darwin) — `jscpd`: ~1,100 dup lines per file | ~50–70% shared |
| `app/drag/CrossWindowDragMonitor.*.tsx` | 454 / 341 / 292 | 311 (win↔linux), 141 (linux↔darwin) | win32 is the outlier; linux/darwin ~half shared |
| `app/hook/useWindowDrag.*.ts` | 278 / 131 / 137 | 325 / 409 | Genuinely different — **not** a finding |
| `app/window/window-controls.*` / `window-header.*.scss` | ≤115 each | small | Legitimately per-platform |

`TileLayout` is the largest single duplication cluster in the frontend — the three files together carry **~3,100 duplicated lines**, the top three entries in `jscpd`'s frontend hot-file list.

**Why it's root cause #3:** the resolver *forces* the fork. A `zoom.ts` with `if (platform === …)` on the one branch that differs — or a `zoom.core.ts` imported by three 10-line platform files — would make the behavioral identity visible instead of hiding it behind three filenames.

### 2.4 Twin primitives built by copy-rename: `mcp` ↔ `skill`

`diff` says these are different files (637 / 1,380 / 189 / 89 differing lines). That is an artifact of the rename — `mcp`→`skill`, `McpServer`→`Skill` — and `jscpd`'s token matching saw through it (~200 dup lines in each `app_api` file, a 53-line clone between the managers). Normalizing the identifiers and re-diffing gives the fair number:

| Layer | Pair | Structurally identical |
|---|---|---|
| Storage | `backend/storage/mcp_servers.rs` (939) ↔ `skills.rs` (1,353) | high (jscpd hot-file: 243 dup lines) |
| RPC | `server/app_api/mcp.rs` (779) ↔ `skill.rs` (684) | **~58%** |
| Manager UI | `view/mcp/mcp-manager.tsx` (250) ↔ `view/skill/skill-manager.tsx` (225) | **~61%** |
| Agent modal | `AgentMcpModal.tsx` (138) ↔ `AgentSkillsModal.tsx` (161) | **~71%** |

Two features, four layers, one shape: *a globally-listable, per-agent-bindable, bundle-referenceable resource with a manager pane and an agent-setup modal*. The third such resource (memory bundles / ABF) is a partial third copy of the same shape.

**Why it's root cause #4:** this is where DRY becomes architecture. The shape is a **managed primitive**; naming it once (a trait + a generic manager/modal parameterized on the resource) turns the fourth primitive into a config entry instead of a fourth copy-rename.

### 2.5 Parallel frameworks that share only vocabulary: saga and reducer in *both* launcher and srv

`SagaCtx`, `SagaOutcome`, `Ctx`, `ServerCtx`, `State`, `ProcessState`, `ProcessRecord`, `EventLog` are each defined independently in **both** `agentmux-launcher` and `agentmux-srv`. Unlike `event_log.rs`, these are **not copies**:

| | launcher | srv | Shared |
|---|---|---|---|
| `saga/` | 6 files, 3,285 lines | 12 files, 5,873 lines | filenames: `mod.rs`, `integration_tests.rs`; method names in `mod.rs`: **only `new`** |
| `reducer/` | 6 files, 5,555 lines | 8 files, 5,839 lines | filename: `window.rs` only |

Two teams (or one team at two times) built the same *idea* twice with the same *names* and different *code*. That is the worst of both: readers assume a shared abstraction that doesn't exist, and a fix to one never reaches the other.

**Why it's root cause #5 and not a refactor ticket:** it needs a decision first. Either (a) there *is* one saga/reducer abstraction and it belongs in a crate both depend on, or (b) they are legitimately different and the shared vocabulary should be **renamed** so nobody reads them as one. Both are defensible; today's state — same names, undocumented divergence, 11K lines — is not.

## 3. Findings — contained

### 3.1 God files

| File | Lines | Note |
|---|---|---|
| `agentmux-srv/src/backend/blockcontroller/persistent.rs` | **7,151** | One controller, **larger than the entire `agentmux-common` crate** (8,202) |
| `agentmux-mcp/src/main.rs` | 4,633 | Single-file crate. Otherwise clean: 53 fns, 4 types, depends on common, re-declares nothing shared — its only problem is being one file |
| `agentmux-srv/src/server/app_api/bundle.rs` | 3,568 | |
| `agentmux-srv/src/server/app_api/mod.rs` | 2,965 | |
| `agentmux-srv/src/identity/resolver/inject.rs` | 2,895 | |
| `frontend/app/view/agent/agent-view.tsx` | 2,730 | Also carries 4 sync comments and hosts the tab-strip logic I extracted into a pure module this week — the extract-and-test pattern already used in that directory is the right tool for the rest of it |
| `frontend/app/view/swarm/swarm-model.ts` | 1,962 | |
| `frontend/app/view/agent/hooks/useAgentCommands.ts` | 1,726 | |

### 3.2 Same-file repetition

- **`frontend/app/view/agent/components/PreLaunchAuthPanel.tsx`** — a **378-line** render tree appears twice (`:239` keyed on `props.accountStatus?.() === "expired"`, `:313` keyed on `controller.state().kind === "expired"`). That is a legacy-prop path and a controller-state path each carrying a full copy of the same JSX — a half-finished migration. Largest single clone in the frontend.
- **`agentmux-cef/src/client/helpers.rs`** — one ~25-line block repeated **seven times** (`:196, :281, :361, :428, :492, :684, :756`); 287 duplicated lines in a 1-file cluster. Reads as a copy-pasted "open socket to srv with timeout, send, read" sequence that wants to be one function with a payload parameter.
- `frontend/app/element/flyoutmenu.tsx` (`:295`/`:497`, `:323`/`:515` — 62 + 68 lines), `frontend/app/view/editor/file-tree.tsx` (`:76`/`:280`, 57 lines), `agentmux-srv/src/server/service/session_restore.rs` (`:504`/`:671`, 45 lines), `tab_move.rs`, `window_mutate.rs`, `backend/history/mod.rs` (three internal clones).

### 3.3 Smaller cross-file pairs worth a look

- `sagas/delete_block.rs:271` ↔ `sagas/tear_off_block.rs:297` (45 lines) — test scaffolding (`test_state()` + `dispatch_apply`), see §3.5.
- `server/agent_handlers/core.rs:751` ↔ `template.rs:602` (41 lines).
- `agentmux-cef/src/client/crash_recovery.rs:291` ↔ `recovery_pages.rs:38` (30 lines).
- `view/native-memory/MemoryAgentFilterBar.tsx:60` ↔ `view/agent/components/AgentPickerFilterBar.tsx:59` (45 lines) — a filter bar built twice.
- `tool-renderers/SearchResults.tsx:27` ↔ `WebFetchResult.tsx:29`; `useAgentActivitySummary.ts:52` ↔ `useNextPromptSuggestion.ts:89`.

### 3.4 Migrations that copy live logic — probably right, but undocumented

`m0020_agent_color_backfill.rs`, `m0021_backfill_agent_bundles.rs`, `m0017_ambient_login_grandfather.rs` carry 30–45-line clones of live code in `def_registry_mirror.rs`, `mcp_servers.rs`, `blockcontroller/core.rs`. Freezing a snapshot of live logic inside a migration is a *legitimate* pattern — a migration must not change meaning when the live code later does. But **no migration or `migrations.rs` states that policy**; a grep for `frozen | snapshot of | must not import | deliberately duplicated` finds nothing about it. Un-annotated, it's indistinguishable from accidental copying, and the next contributor will "helpfully" dedupe it. **Recommendation: write the rule down, not remove the copies.**

### 3.5 Test-fixture builders

`fn make_store` ×9, `shared_store` ×7, `object_store` ×7, `ctx_for` ×7, plus per-module `test_state` / `dispatch_apply`. Low risk, low payoff — but the four largest test files in the workspace (`launcher/reducer/tests.rs` 4,195, `storage/store/tests.rs` 3,624, `server/tests.rs` 3,222, `backend/reactive/tests.rs` 2,707) would all shrink from a `test_support` module. Noted for completeness; not a priority.

## 4. What is already clean (record it, so the plan doesn't "fix" it)

- **Frontend utility helpers are centralized.** Across 677 files, only `formatBytes` is defined twice; `clamp`, `sleep`, `debounce` live once in `frontend/util/util.ts`. The frontend's duplication is at the platform-fork and twin-feature level, not the helper level — the fix is architectural, not a util sweep.
- **`agentmux-mcp` re-declares no shared types** and already depends on `common`. Its 4.6K-line `main.rs` is a splitting job, not a dedup job.
- **The dependency graph is a star.** Nothing depends on anything but `common`. Every lift in §5 is cycle-free by construction.
- **`process_tracker` is a real abstraction** (Windows job / Linux cgroup / macOS pgid behind one trait). The problem is that four other sites *bypass* it, not that it's wrong.
- **The codebase annotates its own duplication.** 334 comments is a liability, but it is also a complete, greppable worklist — the audit's headline findings came from reading what the code already said about itself.

## 5. DRY → modularity: the plan

Ordered by payoff ÷ risk. Each step is independently shippable.

### Phase 1 — mechanical lifts into `agentmux-common` (low risk, immediate)

1. `common::win32` — `CREATE_NO_WINDOW` (and siblings) as `pub const`; delete 21 private copies. *Zero behavior change.*
2. `common::time` — `now_ms()` / `now_secs()`; delete 26 copies. Pick one signature (`i64`), fix the handful of `u64` callers.
3. `common::event_log` — parameterize the log filename; delete one of two 415-line files. The srv copy's own doc comment is the migration guide.
4. `common::process::kill_tree(pid)` — one Windows (`taskkill /F /T /PID`) + one Unix (`libc::kill(-pgid)`) implementation; route the four `taskkill` sites and three `libc::kill` sites through it. `process_tracker` keeps its Job Object path; this is for the non-tracked cases.
5. `agentmux-cef::objc_ffi` — one `extern "C"` block; delete ~55 per-file re-declarations.
6. Move `WindowKind`, `Rect`, `DataPaths` callers onto the `common` definitions that already exist; delete the shadows.

Expected: ~1,200 lines removed, seven sync comments retired, and — more importantly — `common` becomes the obvious home for the next shared thing.

### Phase 2 — codegen the Rust↔TS boundary (medium effort, highest leverage)

7. Generate `gotypes.d.ts` and the 322 RPC stubs from srv `rpc_types` (`Command*` structs already carry the shape; `serde` attributes carry the wire names). A `scripts/gen-rpc-bindings.sh` + a CI gate ("bindings are current," same pattern as the existing "specs index is current" gate) turns 200+ sync comments into a build error.
8. Emit shared *constants* through the same generator — `persistent.rs:785`'s "no shared constant crosses the boundary" becomes false.
9. Move the default layout tree, the provider catalog's `pinned_version`, and the two formatter mirrors (`format_global_brain_block`, `dim_agent_color`) to single-source: backend owns them, frontend receives them over RPC or from generated constants. The layout preset's inability to express 20/80 sizing goes away with it.

### Phase 3 — shared-core platform files (low risk, medium effort)

10. `zoom.core.ts` + three ≤10-line platform files (or one file with a single platform branch — there is exactly one behavioral difference and it is currently zero).
11. `TileLayout.core.tsx` holding the ~50–70% shared body; platform files keep only the drag/DPI/window-edge code that genuinely differs. `CrossWindowDragMonitor` same treatment for the linux/darwin pair; leave win32 alone until measured against the core.
12. Leave `useWindowDrag` and `window-controls` as they are — measured as legitimately different.

### Phase 4 — the managed-primitive abstraction (higher effort, architectural)

13. Extract the `mcp`/`skill` shape: a `ManagedResource` trait on the storage side (global list, per-agent bind, bundle ref, effective-set merge — `effective_skills` / `effective_mcp_servers` are already the same algorithm twice) and a generic manager pane + agent modal on the frontend side parameterized on the resource. Validate by making `skill` the generic path first, then porting `mcp`, then checking whether memory bundles fit. If the third one fits, the abstraction is real; if not, stop at two and the 58–71% duplication is still gone.

### Phase 5 — a decision on saga/reducer (no code until decided)

14. Choose: one shared saga/reducer crate, or rename the launcher's and srv's types so the shared vocabulary stops implying a shared implementation. Write the choice into an ADR. Either outcome is fine; the current state is the only bad one.

### Anytime — contained cleanups

15. `PreLaunchAuthPanel.tsx`: finish the migration to controller state and delete the 378-line legacy render path.
16. `client/helpers.rs`: fold the seven repeats into one function.
17. Split `persistent.rs` (7,151) along the seams it already has (`persistent_resume.rs` and `session_recovery.rs` were split out; continue). Split `agentmux-mcp/src/main.rs` into `tools/` by tool family.
18. Add a one-paragraph policy to `migrations.rs` stating that migrations deliberately freeze copies of live logic and must not be "deduplicated."

## 6. Limits of this audit

- `jscpd` at 60 tokens / 8 lines is a conservative threshold; it under-reports small repeated idioms (3–7 lines) and over-reports boilerplate in trait impls. Every number in §2 was independently re-verified with `diff`, so the headline pairs are solid; the long tail in §3.3 is jscpd-only.
- Similarity for renamed twins (§2.4) used identifier normalization (`mcp|skill → X`); the 58–71% figures are a fair floor, not a ceiling.
- Structure was measured, not runtime — nothing here claims a performance or correctness bug. The one behavioral consequence found (the layout preset that cannot express the backend's sizing) was found by hand this week, not by the tooling.
- Test code was excluded from duplication scans by design; §3.5 is from symbol counts only.
