# Maintenance Section in InstancePanel

**Status:** Spec — not yet implemented  
**Date:** 2026-06-27  
**Placement:** Inside `InstancePanel` (opens when clicking the `v0.49.5` chip in the
bottom-right status bar)  
**Depends on:** PR #1800 (`pending_migrations` in ESTART), `specs/app-update-check.md`

---

## Overview

All maintenance operations live in a single **Maintenance** section appended to the
existing InstancePanel (just before the footer buttons). No new panel is needed.

**Operations covered:**

| # | Operation | Trigger | Duration |
|---|-----------|---------|----------|
| 1 | Code update | User-initiated (UpdateStatus dot → panel) | Varies (download) |
| 2 | Pre-migration DB snapshot | Automatic at startup | 100–500ms |
| 3 | Database migrations | Automatic at startup / on-demand retry | 8ms–5s |
| 4 | Channel pruning | Automatic at launcher startup | < 50ms |
| 5 | Saga log vacuum | On-demand button in panel | < 100ms |

The UpdateStatus dot in the status bar stays as-is (a quick visual indicator). The
InstancePanel is where you *act* — download, install, run migrations, vacuum.

---

## Full Panel Layout

The panel is 320px wide (`POPOVER_WIDTH = 320`). The Maintenance section is inserted
between the Floating panes section and the footer.

```
┌──────────────────────────────────────────────┐  320px
│ Version      v0.49.5                    ⧉   │  header
│ Channel      stable                          │
│ Build        a1b2c3d                   ⧉   │
│ Time         Jun 27, 2026 9:14AM            │
│ Runtime      windows-x64                    │
├──────────────────────────────────────────────┤  divider
│ This process — 2 windows                    │  windows section
│ ● My Workspace                    [this]    │
│   [████████████████████████████░] 100%      │
│ ○ Second Window                             │
│   [████████████████████████████░]  85%      │
├──────────────────────────────────────────────┤  divider
│ Floating panes — 0                          │  panes section
│ No floating panes                           │
├──────────────────────────────────────────────┤  divider
│ Maintenance                                 │  ← NEW SECTION
│ ···                                         │
├──────────────────────────────────────────────┤  divider
│ [+ Open another window]          [Close]    │  footer
└──────────────────────────────────────────────┘
```

---

## ASCII Art: Maintenance Section States

### State A — All clear (nothing pending)

```
├──────────────────────────────────────────────┤
│ Maintenance                                  │
│                                              │
│  ✓  Up to date      v0.49.5                 │
│  ✓  Migrations      11 applied              │
│  ✓  Channels        clean                   │
│  ○  Saga vacuum     last run: Jun 20        │
│                              [Run Vacuum]   │
│                                              │
```

- `✓` green, `○` grey neutral dot.
- "Run Vacuum" is a small inline button (secondary style, same as existing `instance-panel-btn`).
- No badge on the section title.

---

### State B — Code update available

The UpdateStatus dot in the status bar turns green and shows `↓ v0.49.6`. The
Maintenance section also reflects this, with a download action:

```
├──────────────────────────────────────────────┤
│ Maintenance  ↓ 0.49.6                        │
│                                              │
│  ↓  Update v0.49.6 available                │
│     [Download Update]                        │
│                                              │
│  ✓  Migrations      11 applied              │
│  ✓  Channels        clean                   │
│  ○  Saga vacuum     last run: Jun 20        │
│                              [Run Vacuum]   │
│                                              │
```

- Section title badge: `↓ 0.49.6` in accent color.
- "Download Update" is a primary button (filled).
- Other rows still visible below so the panel doesn't change structure dramatically.

---

### State C — Downloading update

```
├──────────────────────────────────────────────┤
│ Maintenance  ↓ 34%                           │
│                                              │
│  ↓  Downloading v0.49.6…  34%              │
│     [████████░░░░░░░░░░░░░░░░░░░░░░]        │
│                                              │
│  ✓  Migrations      11 applied              │
│  ✓  Channels        clean                   │
│  ○  Saga vacuum     last run: Jun 20        │
│                              [Run Vacuum]   │
│                                              │
```

- Badge updates to the percentage.
- Progress bar uses existing CSS variable colors (no hard-coded values).
- "Download Update" button is absent (replaced by the progress bar row).

---

### State D — Update downloaded, ready to restart

```
├──────────────────────────────────────────────┤
│ Maintenance  ↑ ready                         │
│                                              │
│  ↑  v0.49.6 ready to install                │
│     [Restart to Install]                     │
│                                              │
│  ✓  Migrations      11 applied              │
│  ✓  Channels        clean                   │
│  ○  Saga vacuum     last run: Jun 20        │
│                              [Run Vacuum]   │
│                                              │
```

- Badge: `↑ ready` in green.
- "Restart to Install" is a primary button.
- Matches the existing `UpdateStatus.tsx` "ready" state so they agree.

---

### State E — Migrations pending (startup failure)

Startup reported `pending_migrations:3` in ESTART. Existing `BackendStatus.tsx` shows
the static warning; that static block is replaced by a link to open the panel.

```
├──────────────────────────────────────────────┤
│ Maintenance  ⚠ 3 pending                    │
│                                              │
│  ⚠  3 migrations did not apply at startup.  │
│     [Run Migrations]                         │
│                                              │
│  ✓  Channels        clean                   │
│  ○  Saga vacuum     last run: Jun 20        │
│                              [Run Vacuum]   │
│                                              │
```

- Section title badge: `⚠ 3 pending` in warning color.
- Migrations row is absent from the summary table (it's the alert itself).
- "Run Migrations" is a primary button.

---

### State F — Migrations running (live stage list)

User clicked "Run Migrations" (or auto-triggered on-demand). Live events stream from
the CEF subprocess via WPS `upgrade:migration-event`.

```
├──────────────────────────────────────────────┤
│ Maintenance  ·running·                       │
│                                              │
│  ●  Migrations running…          2.4s       │
│     ✓  0011  Shared store backfill   1.1s   │
│     ✓  0012  Agent session index     0.8s   │
│     ·  0013  Transcript index        0.5s   │
│                                              │
│  ✓  Channels        clean                   │
│  ·  Saga vacuum     —                       │
│                                              │
```

Icons:
- `●` blue — parent stage active (pulsing animation)
- `✓` green — sub-step complete
- `·` blue (pulsing) — sub-step currently running

Timers show elapsed seconds, updated every 500ms via `setInterval`.

No cancel button — migrations are not safely interruptible.

---

### State G — Migrations complete

```
├──────────────────────────────────────────────┤
│ Maintenance                                  │
│                                              │
│  ✓  Migrations complete   3 applied         │
│     ✓  0011  Shared store backfill   1.1s   │
│     ✓  0012  Agent session index     0.8s   │
│     ✓  0013  Transcript index        2.8s   │
│                                             │
│  ✓  Channels        clean                   │
│  ○  Saga vacuum     last run: Jun 20        │
│                              [Run Vacuum]   │
│                                              │
```

- Sub-row list collapses after ~5 seconds (or on panel reopen) into the normal
  summary row: `✓  Migrations  14 applied`.
- Badge clears.

---

### State H — Migration failed

```
├──────────────────────────────────────────────┤
│ Maintenance  ✗ 1 failed                      │
│                                              │
│  ✗  Migration 0012 failed                   │
│     ✓  0011  Shared store backfill   1.1s   │
│     ✗  0012  Agent session index     1.0s   │
│             foreign key constraint failed    │
│     Check ~/.agentmux/logs/ for details.     │
│     [Retry]                                  │
│                                              │
│  ✓  Channels        clean                   │
│  ○  Saga vacuum     last run: Jun 20        │
│                              [Run Vacuum]   │
│                                              │
```

- Badge: `✗ 1 failed` in error color.
- Error text (one line from the JSON event `error` field) appears under the failed row.
- "Retry" button re-triggers the same on-demand migration flow.

---

### State I — Saga vacuum running

```
├──────────────────────────────────────────────┤
│ Maintenance                                  │
│                                              │
│  ✓  Up to date      v0.49.5                 │
│  ✓  Migrations      11 applied              │
│  ✓  Channels        clean                   │
│  ·  Saga vacuum     running…                │
│                                              │
```

- `·` pulsing blue dot.
- "Run Vacuum" button replaced by "running…" text.
- Returns to normal state (State A) once the command resolves, with a transient:

---

### State J — Saga vacuum done (transient, ~3s)

```
├──────────────────────────────────────────────┤
│ Maintenance                                  │
│                                              │
│  ✓  Up to date      v0.49.5                 │
│  ✓  Migrations      11 applied              │
│  ✓  Channels        clean                   │
│  ✓  Saga vacuum     247 rows removed        │
│                              [Run Again]    │
│                                              │
```

After 3 seconds the `✓  Saga vacuum  last run: Jun 27  [Run Vacuum]` row replaces it.

---

### State K — Channel prune not yet implemented (placeholder row)

While `SPEC_LOCAL_CHANNEL_PRUNER_2026_06_25.md` is unimplemented, the Channels row
shows a neutral dash:

```
│  —  Channels        not yet checked         │
```

Once the pruner lands, it updates the row on first run to show:
```
│  ✓  Channels        2 removed (143 MB)      │
```
or
```
│  ✓  Channels        clean (0 dead)          │
```

---

## BackendStatus.tsx Change

The static migration-failure block (lines 198–213) becomes a button that opens the
InstancePanel. The InstancePanel itself shows the Maintenance section which has the
"Run Migrations" call-to-action.

**Before (current):**
```tsx
<div class="status-bar-popover-row">
    <span>Migration failed at startup — check logs, then restart to retry.</span>
</div>
```

**After:**
```tsx
<Show when={(backendInfo()?.pending_migrations ?? 0) > 0}>
    <div class="status-bar-popover-divider" />
    <div class="status-bar-popover-row">
        <span style={{ color: "var(--warning-color)" }}>
            ⚠ {backendInfo()!.pending_migrations} migrations pending
        </span>
    </div>
    <div class="status-bar-popover-row">
        <button class="status-bar-restart-btn" onClick={() => {
            setPopoverOpen(false);
            openVersionPanel();   // triggers StatusBar to open InstancePanel
        }}>
            Open Maintenance ↗
        </button>
    </div>
</Show>
```

The `openVersionPanel()` function calls up to `StatusBar` via a callback prop or
stores a signal — exact wiring TBD in implementation.

---

## Data Flow

### Code update

```
App launch + 10s
  → agentmux-srv: GET github.com/agentmuxai/agentmux/releases/latest
  → WPS "app-update-status" event → UpdateStatus.tsx (dot in status bar)
  → Also: InstancePanel Maintenance section reads same atom

User clicks [Download Update] in InstancePanel
  → getApi().installAppUpdate()   (currently stubbed in stubs.rs)
  → CEF: download to temp, emit WPS "app-update-status" { state: "downloading", pct }
  → InstancePanel: live progress bar update

Download complete
  → WPS "app-update-status" { state: "ready" }
  → InstancePanel: [Restart to Install] button

User clicks [Restart to Install]
  → getApi().installAppUpdate() with { phase: "install" }
  → CEF: launch installer / replace binary → exit
```

### Post-restart migrations (automatic)

```
Launcher starts agentmux-srv
  → srv: maybe_snapshot_pre_migration() → ~/.agentmux/snapshots/
  → srv: count_pending_migrations() > 0 → emit AGENTMUXSRV-MIGRATING to stderr
  → launcher: extend ESTART deadline to 30 min
  → srv: run_pending_migrations() → JSON events to stderr
      { "event": "migration_start", "id": "0013", "label": "..." }
      { "event": "migration_done",  "id": "0013", "duration_ms": 2800 }
      { "event": "complete", "applied": 3, "skipped": 11 }
  → launcher: parse events → (currently: splash stage list)
  → srv: emit AGENTMUXSRV-ESTART pending_migrations:0   (or N on failure)
  → frontend: backendInfo().pending_migrations determines Maintenance section state
```

### On-demand migration retry (user clicks "Run Migrations")

```
User clicks [Run Migrations] in InstancePanel
  → getApi().runMigrations()
  → CEF "run_migrations" command (backend.rs)
  → Guard: Mutex<bool> — reject if already running
  → Spawn: agentmux-srv --wavedata <path> migrate  (stdout piped)
  → Background task: read stdout line-by-line
      → each JSON line → WPS "upgrade:migration-event" (scope: local)
  → InstancePanel: waveEventSubscribe → State F live stage list
  → On process exit:
      success → update state.pending_migrations → 0
                emit WPS "upgrade:migrations-complete"
                InstancePanel → State G (complete)
      failure → emit WPS "upgrade:migrations-failed" { error, failed_id }
                InstancePanel → State H (failed)
```

### Saga vacuum

```
User clicks [Run Vacuum]
  → getApi().runSagaVacuum()
  → CEF "run_saga_vacuum" command
  → spawn_blocking: saga_log.vacuum_older_than(cutoff)
  → return { rows_deleted: N }
  → InstancePanel → State J (done, transient)
  → after 3s → State A with updated "last run" timestamp
```

---

## WPS Events (new)

**File:** `frontend/app/store/wps-events.ts`

```typescript
UpgradeMigrationEvent:     "upgrade:migration-event",
UpgradeMigrationsComplete: "upgrade:migrations-complete",
UpgradeMigrationsFailed:   "upgrade:migrations-failed",
UpgradeSagaVacuumDone:     "upgrade:saga-vacuum-done",
```

### `upgrade:migration-event` payload

```typescript
interface MigrationProgressEvent {
    kind: "start" | "done" | "error" | "complete";
    id?: string;            // migration id (start / done / error)
    label?: string;         // human label (start)
    duration_ms?: number;   // elapsed (done / complete)
    applied?: number;       // total applied (complete)
    skipped?: number;       // already-applied count (complete)
    error?: string;         // one-line error detail (error)
}
```

---

## Implementation Plan

### Phase 1 — CEF backend commands

**`agentmux-cef/src/commands/backend.rs`**

**`run_migrations(state)`:**
1. Lock `state.migration_running: Mutex<bool>` — return early if already locked
2. Resolve `agentmux-srv` path (reuse `resolve_srv_binary`)
3. Spawn `agentmux-srv --wavedata <path> migrate` with `stdout: Stdio::piped()`
4. Tokio task: read stdout line-by-line, forward each valid JSON line as WPS
   `upgrade:migration-event` via the existing `emit_wps_event` helper
5. On exit:
   - success: set `state.pending_migrations = 0`, emit `upgrade:migrations-complete`
   - failure: emit `upgrade:migrations-failed { error, failed_id }`
6. Return `Ok(json!({"started": true}))` immediately (non-blocking)

**`run_saga_vacuum(state)`:**
1. Read saga DB path from `state.data_paths.saga_db`
2. `tokio::task::spawn_blocking(|| vacuum_older_than(cutoff))`
3. Return `Ok(json!({"rows_deleted": N}))`

**`agentmux-cef/src/ipc.rs`** — add routes:
```rust
"run_migrations"  => commands::backend::run_migrations(state.clone()).await,
"run_saga_vacuum" => commands::backend::run_saga_vacuum(state.clone()).await,
```

### Phase 2 — WPS event constants

**`frontend/app/store/wps-events.ts`** — add the 4 constants listed above.

### Phase 3 — Frontend API

**`frontend/util/cef-api.ts`:**
```typescript
runMigrations: async () =>
    invokeCommand<{ started: boolean }>("run_migrations"),
runSagaVacuum: async () =>
    invokeCommand<{ rows_deleted: number }>("run_saga_vacuum"),
```

**`frontend/types/custom.d.ts`** — add both methods to the `AppApi` interface.

### Phase 4 — Maintenance section component

**`frontend/app/statusbar/MaintenanceSection.tsx`** (new)

Props: none — reads global atoms directly.

Internal state machine (SolidJS signals):

```typescript
type MigrationState =
  | { kind: "idle"; pendingCount: number }
  | { kind: "running"; steps: MigrationStep[] }
  | { kind: "complete"; steps: MigrationStep[]; applied: number }
  | { kind: "failed"; steps: MigrationStep[]; error: string; failedId: string };

type UpdateState =
  | { kind: "none" }
  | { kind: "available"; version: string }
  | { kind: "downloading"; version: string; pct: number }
  | { kind: "ready"; version: string };

type VacuumState =
  | { kind: "idle"; lastRunMs: number | null }
  | { kind: "running" }
  | { kind: "done"; rowsDeleted: number; ranAtMs: number };
```

- Subscribes to `upgrade:migration-event`, `upgrade:migrations-complete`,
  `upgrade:migrations-failed`, `upgrade:saga-vacuum-done` via `waveEventSubscribe`
- Reads `backendInfo().pending_migrations` from global atoms for initial state
- Reads the `app-update-status` atom (same source as `UpdateStatus.tsx`)
- `setInterval` (500ms) for elapsed timers while `migrationState.kind === "running"`
- Vacuum "done" state reverts after 3000ms via `setTimeout`

**`frontend/app/statusbar/MaintenanceSection.scss`** (new)

Classes: `.maintenance-section`, `.maintenance-row`, `.maintenance-sub-row`,
`.maintenance-icon`, `.maintenance-timer`, `.maintenance-badge`, `.maintenance-btn`

Follow the existing InstancePanel CSS variable conventions (`--main-text-color`,
`--secondary-text-color`, `--warning-color`, `--error-color`, `--accent-color`,
`--highlight-bg`, `--border-color`).

### Phase 5 — Wire into InstancePanel

**`frontend/app/statusbar/InstancePanel.tsx`:**

1. Import `MaintenanceSection`
2. Add a `<div class="instance-panel-divider" />` after the floating panes section
3. Add `<div class="instance-panel-section"><MaintenanceSection /></div>`

The existing footer (`+ Open another window` + `Close`) stays last.

### Phase 6 — BackendStatus.tsx cleanup

Replace the static migration-failure text (lines 198–213) with the
`⚠ N migrations pending  [Open Maintenance ↗]` row described above.

The "Open Maintenance ↗" button needs a way to programmatically open the
InstancePanel. Options (pick during implementation):
- Lift InstancePanel open-state to `StatusBar.tsx` and pass a setter down to
  `BackendStatus.tsx` via context or prop
- Emit a custom DOM event that `StatusBar.tsx` listens for (simpler, no prop
  threading)
- Signal in a tiny `maintenancePanelOpenAtom` in a new store file

### Phase 7 — Code update (implement `install_update`)

Wire the existing `install_update` stub per `specs/app-update-check.md`:
1. Detect install type (NSIS / portable / AppImage / DMG / MSIX / unknown)
2. Download platform asset to temp / Downloads, stream `pct` progress as
   `app-update-status` events (already consumed by `UpdateStatus.tsx`)
3. On "ready": launch installer (NSIS, DMG) or replace binary + exit (portable,
   AppImage)

---

## Edge Cases

| Case | Handling |
|------|----------|
| Panel closed mid-migration | CEF subprocess continues. WPS events still fire. On reopen the component reads `pending_migrations` from `backendInfo()` to decide initial state — if still > 0 it shows State E |
| Double-click "Run Migrations" | `Mutex<bool>` in CEF rejects the second call; button disabled while `migrationState.kind === "running"` |
| Migrations already current | `agentmux-srv migrate` emits `{"event":"complete","applied":0,"skipped":14}` instantly; panel skips to State G without showing an empty stage list |
| Migration partial then process killed | Migrations are idempotent; retry resumes from the first unapplied ID |
| Saga vacuum: nothing to delete | `rows_deleted: 0` → show "Nothing to clean up" instead of "0 rows removed" |
| Channel pruner not implemented | Row shows `— Channels  not yet checked` (neutral dash, no warning) |
| Snapshot row | Not shown in the panel summary — it's a silent background operation with no user action. If users want to find snapshots they check `~/.agentmux/snapshots/` |
| UpdateStatus.tsx conflict | Both UpdateStatus and MaintenanceSection read the same `app-update-status` atom. They stay in sync by construction. UpdateStatus stays in the status bar as the quick indicator; InstancePanel has the action buttons |

---

## Files to Create / Modify

| File | Action |
|------|--------|
| `agentmux-cef/src/commands/backend.rs` | Add `run_migrations`, `run_saga_vacuum` |
| `agentmux-cef/src/ipc.rs` | Route both commands |
| `agentmux-cef/src/commands/stubs.rs` | Remove `run_migrations` / `run_saga_vacuum` stubs if present; keep `install_update` stub until Phase 7 |
| `frontend/app/store/wps-events.ts` | Add 4 `upgrade:*` event constants |
| `frontend/util/cef-api.ts` | Add `runMigrations`, `runSagaVacuum` |
| `frontend/types/custom.d.ts` | Add both to `AppApi` interface |
| `frontend/app/statusbar/MaintenanceSection.tsx` | New component |
| `frontend/app/statusbar/MaintenanceSection.scss` | New styles |
| `frontend/app/statusbar/InstancePanel.tsx` | Add `<MaintenanceSection />` before footer |
| `frontend/app/statusbar/BackendStatus.tsx` | Replace static migration text with "Open Maintenance ↗" button |
| `agentmux-cef/src/commands/stubs.rs` | Remove `install_update` stub (Phase 7 only) |
