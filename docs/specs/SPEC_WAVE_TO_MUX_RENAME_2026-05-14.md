# SPEC: `Wave*` → `Mux*` rename (purge Wave Terminal branding)

**Date:** 2026-05-14
**Author:** AgentX
**Status:** ❌ **Deferred — not worth refactoring at this time** (decision 2026-05-14, mid-implementation)
**Decision context:** The mechanical sweep was attempted (~150 files modified, sweeps for symbols / strings / module paths / SQL schema / env vars). Build verification surfaced ~30 remaining cross-references that needed careful handling (path-prefix resolution like `wps::Foo` vs `::wps::`, relative imports, comments, etc.). Each successive grep pass found more straggler references. The cost-benefit flipped — the rename is purely cosmetic (it removes leftover Wave Terminal branding but changes no behavior) and the sweep was getting deeper than the value justified.
**Outcome:** All 5 `Wave*` types stay (`WaveObj`, `WaveObjUpdate`, `WaveStore`, `WaveWindow`, etc.). Module names stay (`wos.ts`, `wps.ts`, `wstore.rs`, `wps.rs`, `gotypes.d.ts`). Wire string `"waveobj:update"` stays. SQLite table `db_wave_file` stays. Env vars stay. The bridge PR (`SPEC_OBJ_UPDATE_BRIDGE_2026-05-14.md`) ships on the existing Wave* names.
**If revisited later:** the analysis below is still valid — the surface area, the migration shape, the open questions are all resolved. Pick this up if/when it stops feeling like cosmetic churn (e.g. as part of a broader "kill all Wave Terminal artifacts" effort, or alongside a real architectural change that touches the same files anyway).

---

## 1. Goal (deferred — kept for record)

Replace the `Wave*` type prefix (leftover from the Wave Terminal fork) with `Mux*` across Rust + TypeScript + module names. No semantic change — purely a rename so the codebase reads like an AgentMux codebase, not a Wave Terminal one.

This is a pre-flight cleanup so the upcoming bridge PR (`SPEC_OBJ_UPDATE_BRIDGE_*`) can ship on clean naming from day one.

---

## 2. Scope (measured)

```bash
grep -rE "WaveObj|WaveStore" agentmux-{srv,cef,common}/src frontend | wc -l   # 386
grep -rEl "WaveObj|WaveStore" agentmux-{srv,cef,common}/src frontend | wc -l  # 67 files
```

**11 distinct `Wave*` types** detected (`grep -rE "\bWave[A-Z][a-zA-Z]+" agentmux-srv/src agentmux-common/src`):

```
WaveEvent
WaveEvents
WaveFile
WaveFiles
WaveInfoData
WaveLock
WaveNotificationOptions
WaveObj
WaveObjUpdate
WaveStore
WaveWindow
```

Plus module/file/function names:

- `frontend/app/store/wos.ts` (module — `WOS` namespace import everywhere)
- `agentmux-srv/src/backend/storage/store.rs`
- `wave_obj_to_value()` (helper function)
- `getWaveObjectAtom`, `getWaveObjectValue` (frontend helpers)
- `wpsSubscribeToObject` (the `wps` prefix — possibly "wave pub/sub")
- `WAS` / `WAV` if any (didn't find but worth grepping)

---

## 3. Rename table

| Old | New | Notes |
|---|---|---|
| **Type definitions (Rust + TS)** | | |
| `WaveObj` (trait, Rust) | `MuxObj` | `agentmux-srv/src/backend/obj.rs:121-138` |
| `WaveObjUpdate` (struct, Rust + TS wire type) | `MuxObjUpdate` | The wire format; both sides must match in the same PR |
| `WaveStore` (Rust) | `MuxStore` | `agentmux-srv/src/backend/storage/store.rs` |
| `WaveWindow` (TS type) | `MuxWindow` | In `frontend/types/gotypes.d.ts` |
| `WaveEvent`, `WaveEvents` | `MuxEvent`, `MuxEvents` | |
| `WaveFile`, `WaveFiles` | `MuxFile`, `MuxFiles` | |
| `WaveInfoData` | `MuxInfoData` | |
| `WaveLock` | `MuxLock` | |
| `WaveNotificationOptions` | `MuxNotificationOptions` | |
| **Helper functions** | | |
| `wave_obj_to_value()` | `mux_obj_to_value()` | |
| `getWaveObjectAtom` | `getMuxObjectAtom` | |
| `getWaveObjectValue` | `getMuxObjectValue` | |
| `reloadWaveObject` | `reloadMuxObject` | |
| `loadAndPinWaveObject` | `loadAndPinMuxObject` | |
| `updateWaveObject` (internal) | `updateMuxObject` | |
| `wpsSubscribeToObject`, `wpsReconnectHandler` | `mpsSubscribeToObject`, `mpsReconnectHandler` | **Confirmed:** `wps` stands for **"Wave PubSub"** (per the module's structure in `agentmux-srv/src/backend/wps.rs:60` and `frontend/app/store/wps.ts:146`). Rename to `mps` ("Mux PubSub") for consistency with `mstore` / `mos`. |
| **Module / file names** | | |
| `frontend/app/store/wos.ts` | `frontend/app/store/mos.ts` | All importers update path |
| `WOS` namespace import (`import * as WOS from ...`) | `MOS` | 19 importers |
| `agentmux-srv/src/backend/storage/store.rs` | `agentmux-srv/src/backend/storage/mstore.rs` | Module declaration + 50+ `use` statements |
| `agentmux-srv/src/backend/wps.rs` | `agentmux-srv/src/backend/mps.rs` | Wave PubSub module → Mux PubSub. Sweep `use ...::wps::...` across crate |
| `frontend/app/store/wps.ts` | `frontend/app/store/mps.ts` | Same; 6 frontend importers |
| `frontend/types/gotypes.d.ts` | `frontend/types/srv-types.d.ts` | 2074-line type bindings file; the filename used to reference the (now-defunct) Go source. New name describes where the types come from today: the `agentmux-srv` sidecar. The file is in the global ambient-type lookup (`declare global { ... }`), so renaming doesn't require import updates anywhere — just rename the file. `tsconfig.json` uses `include: ["frontend/**/*"]` so it auto-discovers. |
| **Wire-protocol strings** | | |
| `"waveobj:update"` (WebSocket eventtype discriminator) | `"muxobj:update"` | **5 occurrences:** `agentmux-srv/src/server/app_api.rs:399,816`; `service.rs:45`; `websocket.rs:125,541` (plus frontend handler in `app/store/global.ts`). Backend + frontend must change in the same commit. See §10.1 for migration concerns. |
| **Database schema** | | |
| `db_wave_file` (SQLite table) | `db_mux_file` | **11 SQL statements** reference this table name (CREATE, INSERT, SELECT, UPDATE, DELETE, INDEX). Requires a schema migration — see §11.2. |
| **Runtime env / globals** | | |
| `__WAVE_SERVER_WS_ENDPOINT__` (window global) | `__MUX_SERVER_WS_ENDPOINT__` | Injected by launcher/CEF at boot. See §10.3 for deprecation strategy. |
| `__WAVE_SERVER_WEB_ENDPOINT__` | `__MUX_SERVER_WEB_ENDPOINT__` | Same. |
| **Constants** | | |
| `OTYPE_*` (e.g. `OTYPE_WORKSPACE`) | unchanged | These don't have `Wave` in the name; keep |
| **Comments and docs** | | |
| `WaveStore: generic OID-based CRUD for WaveObj types.` (`wstore.rs:1`) | `MuxStore: generic OID-based CRUD for MuxObj types.` | Update the doc comments too |
| `// generated by cmd/generate/main-generatets.go` (`gotypes.d.ts:4`) | Remove — stale; no Go file exists in repo (no codegen pipeline today). Add `// Hand-maintained type bindings; keep in sync with agentmux-srv/src/backend/obj.rs` instead. | Side cleanup |

### 3.1 Names NOT to rename (out of scope)

- `OTYPE_*` constants — they're already brand-neutral.
- `WOS` references in **comments and old log lines** if any exist in archived specs / changelog — leave history alone.
- External-protocol field names (e.g. `oref`, `oid`, `otype`) — those are wire format; renaming is a separate breaking change.
- File names of historical specs that have `Wave` in them — historical record.

---

## 4. File renames (with `git mv`)

| Old path | New path |
|---|---|
| `frontend/app/store/wos.ts` | `frontend/app/store/mos.ts` |
| `agentmux-srv/src/backend/storage/store.rs` | `agentmux-srv/src/backend/storage/mstore.rs` |
| `agentmux-srv/src/backend/storage/store.test.rs` (if exists) | `mstore.test.rs` |

Use `git mv` so the rename history is preserved across blame.

After renames, update:

- `mod wstore;` → `mod mstore;` in the parent `mod.rs` (Rust)
- All `use ...::wstore::...` paths
- All TS imports — sweep is mechanical (`@/store/wos` → `@/store/mos`, `@/app/store/wos` → `@/app/store/mos`)

---

## 5. PR sequencing (within this PR)

Do all renames in **one commit**, not many small ones. Reason: a `WaveObj` partial-rename build is broken; PR reviewers and bisect want a single coherent transition.

Order of operations inside the commit:

1. `git mv` the files.
2. Sweep Rust source: `sed -i 's/WaveObj/MuxObj/g; s/WaveStore/MuxStore/g; ...'` across `agentmux-{srv,cef,common}/src` and `frontend`.
3. Update doc comments.
4. Update `gotypes.d.ts` (stale codegen comment + the `WaveWindow` type name).
5. `cargo check --workspace` + `npx tsc --noEmit` until clean.
6. Run existing tests (`npm test`, `cargo test -p agentmux-srv`).

The bump CLI runs at the end as usual.

---

## 6. Why one big commit, not staged

Tempting to think "rename Rust first, then frontend in a second commit" — but the wire types (`WaveObjUpdate`) are shared. A staged commit would have either the Rust or the TS side referring to a name that no longer exists. Single-commit transitions are easier to revert if something breaks in CI.

The cost (one big diff) is offset by the fact that the diff is purely mechanical and reviewable as such.

---

## 7. Test plan

### 7.1 Build verification

- [ ] `cargo check --workspace` clean
- [ ] `cargo build --release -p agentmux-srv -p agentmux-cef -p agentmux-launcher -p agentmux-bashwrap -p agentmux-common`
- [ ] `npx tsc --noEmit -p tsconfig.json` clean
- [ ] `npm run build:dev` (frontend prod bundle) succeeds
- [ ] `npm run build:prod` succeeds
- [ ] `npx vitest run` — full frontend test suite passes
- [ ] `cargo test --workspace` passes

### 7.2 Functional verification

This is a pure rename — there should be ZERO behavioral changes. The verification is:

- [ ] `task dev` boots, window opens, frontend loads
- [ ] Workspace + tab + block + window operations work exactly as before
- [ ] InstancePanel renames a window successfully
- [ ] Existing PR #841 reactive title still works
- [ ] No new warnings beyond pre-existing ones (run `cargo build` twice, diff warnings)

### 7.3 Grep verification

After the commit, these should return zero hits (or only intentional history/comment mentions):

```bash
grep -rE "\bWave[A-Z]" agentmux-srv/src agentmux-cef/src agentmux-common/src frontend
grep -rE "WaveObj|WaveStore|WaveWindow" agentmux-srv/src agentmux-cef/src agentmux-common/src frontend
grep -rE "from \"@/store/wos\"|from \"@/app/store/wos\"" frontend
```

---

## 8. Risks

| Risk | Mitigation |
|---|---|
| Missed references → build error | Comprehensive grep before commit; CI catches what grep misses |
| External dependency uses `Wave*` (e.g. another repo, e2e tests) | Search a5af-org for `WaveObj` / `WaveStore` callers; coordinate or stage |
| `WaveWindow` mentioned in user-facing docs (agentmux-docs) | Coordinate a docs PR in lockstep (cite `agentmux-docs` PR template) |
| Bisect/blame disruption | `git mv` preserves rename for blame; large mechanical diff is annotated as such in PR title |
| Stale CI cache → old types persist in dist/ | `task clean` before package build |
| `task package` outputs include old name in binary metadata (file properties, About modal) | About modal text was already brand-neutral; check |
| Performance regression hiding in the diff | Mechanical rename has no runtime impact; perf tests unchanged |

### 8.1 What could go wrong despite all the care

A type name appearing inside a **string literal** (e.g. logged as `"WaveObj"`, used as a JSON discriminator field value) won't be caught by symbol rename. Need a separate grep pass for string mentions:

```bash
grep -rE '"WaveObj|"WaveStore|"WaveWindow' agentmux-{srv,cef,common}/src frontend
```

If any hits exist, those are protocol/wire mentions and may require coordinated handling (e.g. a backwards-compat alias for one release).

---

## 9. Coordination with other PRs

### 9.1 The bridge PR (`SPEC_OBJ_UPDATE_BRIDGE_*`)

This rename PR is **sequenced before** the bridge PR. Update the bridge spec post-merge of this PR to use the new names (`MuxObj`, `MuxObjUpdate`, `MuxStore`, `mux_obj_to_value`). Already noted in the bridge spec §13 implementation readiness.

### 9.2 The docs repo (`agentmux-docs`)

The docs glossary still references `WaveObj` as a type concept implicitly through `WaveWindow`. Open a coordinated docs PR in lockstep that:
- Updates `glossary.md` to use `MuxWindow` if it ever appears (unlikely — the glossary mostly talks about "window" the concept)
- Doesn't drag in the `Wave*` legacy mention currently in the docs (search for it)

### 9.3 External consumers

If any e2e test repos or agent SDK packages import types named `WaveObj` from `@a5af/...`, those need updates. Discovery: `gh search code 'WaveObj' --owner a5af`.

---

## 10. Resolved questions (research log)

These were open in the first draft of this spec; resolved by reading the source. Kept here so the next reader doesn't have to re-investigate.

### 10.1 Wire-protocol string `"waveobj:update"` — RESOLVED ✓ Lockstep rename

**Question:** Is the `"waveobj:update"` WebSocket eventtype safe to rename?

**Answer:** **Yes, in the same commit as the type rename.** The string is a discriminator in the WS message envelope, present at:
- Rust backend: `agentmux-srv/src/server/{app_api.rs:399,816, service.rs:45, websocket.rs:125,541}` — 5 emit sites
- Frontend: `frontend/app/store/global.ts` — listener handler

Both ends ship from this repo in a single release. The rename to `"muxobj:update"` is safe as a single-commit lockstep change — there's no external consumer of the WS protocol (no mobile client, no third-party SDK that subscribes).

**One subtle migration concern:** if a user runs an older portable + the new build simultaneously and they ever talked to each other (they don't — each instance has its own backend), the protocol mismatch would matter. Since instances are isolated, this is a non-issue.

### 10.2 SQLite table `db_wave_file` — RESOLVED ✓ Schema migration required

**Question:** Is renaming the `db_wave_file` SQLite table safe?

**Answer:** **Requires a SQLite schema migration**, but the codebase already has a migration framework for this. The 11 SQL statements that reference the table all live in `agentmux-srv/src/backend/storage/filestore/` (filestore implementation).

**Migration approach:**

```sql
-- Migration script (new schema version)
ALTER TABLE db_wave_file RENAME TO db_mux_file;
-- Also rename any indexes:
ALTER INDEX idx_db_wave_file_* RENAME TO idx_db_mux_file_*;  -- if any
```

The rename PR must:
1. Bump the schema version constant (locate via grep `SCHEMA_VERSION` or similar in `agentmux-srv/src/backend/storage/`).
2. Add a migration entry that renames the table on existing databases.
3. Update all 11 SQL statements to reference `db_mux_file`.
4. Verify by running against an existing database from a prior version — the migration should be idempotent and lossless.

**Why the type name `WaveFile` is separate from the table name:** The Rust struct in `filestore/types.rs:39` is the in-memory representation; the SQL is the on-disk representation. Both rename together for consistency, but they're independent string surfaces — both must be updated in the rename PR.

### 10.3 Env vars `__WAVE_SERVER_*` — RESOLVED ✓ Internal-only, rename in-place

**Question:** Are `__WAVE_SERVER_WS_ENDPOINT__` and `__WAVE_SERVER_WEB_ENDPOINT__` window globals safe to rename, or are they external-facing?

**Answer:** **Internal-only.** These are runtime-injected globals on `window` set by the CEF host as part of the initial page bootstrap. Their consumers are:
- The renderer process's `bootstrap.ts` / equivalent (reads them once, then passes resolved values into the app)
- No documented external consumer (no extension SDK, no agent-side runtime that reads them)

**Decision:** rename in-place to `__MUX_SERVER_WS_ENDPOINT__` / `__MUX_SERVER_WEB_ENDPOINT__` in the same commit. No backward-compat alias needed — both setter (Rust host code) and reader (frontend bootstrap) update together.

If we discover during the PR that an external consumer DOES read these (e.g. a debug tool, a test helper), add an alias:
```js
window.__MUX_SERVER_WS_ENDPOINT__ = window.__WAVE_SERVER_WS_ENDPOINT__ = "...";  // dual-name for one release
```
…and deprecate the old name in a follow-up.

### 10.4 `wps` prefix — RESOLVED ✓ Stands for "Wave PubSub", rename to `mps`

**Question:** What does `wps` stand for, and should it rename to `mps`?

**Answer:** **"Wave PubSub"** — confirmed by inspecting the module at `agentmux-srv/src/backend/wps.rs:60` (defines `WaveEvent` and the pub/sub broker, ~800 lines) and `frontend/app/store/wps.ts:146` (exports the matching JS-side handler).

Rename to `mps` for consistency. The files `wps.rs` and `wps.ts` also rename (see §3 rename table). 6 frontend files import from `@/store/wps` — mechanical sed sweep.

### 10.5 `gotypes.d.ts` codegen — RESOLVED ✓ Hand-maintained, safe to rename

**Question:** Is `gotypes.d.ts` regenerated by some hidden codegen pipeline that would overwrite our changes?

**Answer:** **No codegen pipeline exists.** Verified:
- `find . -name "*.go"` returns 0 results — the file's `// generated by cmd/generate/main-generatets.go` header is stale.
- No Taskfile entry, no npm script, no Cargo build script references `gotypes`, `generate-ts`, `ts-rs`, `specta`, or `typeshare`.
- `tsconfig.json` uses `include: ["frontend/**/*"]` — the file is auto-discovered, no explicit path lock.

Safe to:
- Rename file to `srv-types.d.ts` (the user-chosen name).
- Update the stale header to `// Hand-maintained type bindings; keep in sync with agentmux-srv/src/backend/{obj.rs, rpc_types.rs, wps.rs}`.

### 10.6 Other hidden references — RESOLVED ✓ Audit clean

| Category | Hits | Action |
|---|---|---|
| Cargo features `WAVE_*` | none | OK |
| npm package names | none | OK |
| CSS classes `.wave-*` | none | OK |
| localStorage / sessionStorage keys | none | OK |
| File extensions / lock filenames | none | OK |
| HTTP route paths | one handler `handle_wave_file` (function name only; URL path `/agentmux/file` is brand-neutral) | Rename function via the type rename sweep |
| Activity telemetry fields `waveaifgminutes`, `waveaiactiveminutes` | in `gotypes.d.ts` field names | **Out of scope** — these are backwards-compat activity-tracking field names that would break analytics dashboards if renamed. Leave for a separate analytics-schema-migration PR. |

---

## 11. Naming sanity check (all 9 `Wave*` types verified)

| Type | Definition site (verified) |
|---|---|
| `WaveEvent` | `agentmux-srv/src/backend/wps.rs:60` |
| `WaveFile` | `agentmux-srv/src/backend/storage/filestore/types.rs:39` |
| `WaveLock` | `agentmux-srv/src/backend/base.rs:189` |
| `WaveInfoData` | `agentmux-srv/src/backend/rpc_types.rs:1106` |
| `WaveNotificationOptions` | `agentmux-srv/src/backend/rpc_types.rs:1145` |
| `WaveObj` (trait) | `agentmux-srv/src/backend/obj.rs:121` |
| `WaveObjUpdate` (struct) | `agentmux-srv/src/backend/obj.rs:468` |
| `WaveStore` | `agentmux-srv/src/backend/storage/store.rs:24` |
| `WaveWindow` (TS) | `frontend/types/gotypes.d.ts` (global) |

All 9 accounted for, each with a single definition site (no duplicate-define traps).

---

## 12. Implementation readiness checklist

Pre-flight, before opening the rename PR:

- [x] All 9 `Wave*` type definitions verified at single sites (§11)
- [x] Wire-protocol string `"waveobj:update"` rename scoped + lockstep (§10.1)
- [x] SQLite schema migration plan documented (§10.2)
- [x] Env-var rename strategy decided (§10.3)
- [x] `wps` prefix expansion confirmed (§10.4)
- [x] `gotypes.d.ts` codegen status verified (§10.5)
- [x] Hidden-reference audit complete (§10.6)
- [x] File renames identified with `git mv` (§4)
- [ ] Locate `SCHEMA_VERSION` constant in `agentmux-srv/src/backend/storage/` — needed for §11.2 migration
- [ ] Confirm migration framework supports `ALTER TABLE ... RENAME TO ...` (SQLite 3.25+ does)

The two unchecked items are 5-minute lookups during the PR, not spec blockers.
