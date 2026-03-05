# Spec: Bottom Status Bar

## Overview

A persistent, single-line status bar anchored to the bottom of the app window. It surfaces ambient system state — backend health, active connections, update status — without interrupting the user's workflow. Read-only. Non-interactive by default; individual items may be clickable to reveal detail.

---

## Goals

- Surfaces operational state that currently has no persistent home (backend status, connection health)
- Removes update/config-error noise from the title header (those indicators move here)
- Matches the aesthetic of the existing dark theme — subtle, low-contrast by default, high-contrast only when action is needed
- Zero layout impact on the main content area (fixed height, no reflow)

---

## Layout & Positioning

```
┌─────────────────────────────────────────────────────────────────┐
│  Window Header                                                  │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  Main Content (workspace, tabs, blocks)                         │
│                                                                 │
├─────────────────────────────────────────────────────────────────┤
│  ● Backend  ■ 2 connections  ⚠ Config error     v0.31.38  ↑    │  ← Status Bar
└─────────────────────────────────────────────────────────────────┘
```

- **Position**: Fixed bottom of `.app-content` (inside window, below workspace)
- **Height**: 22px (scales with `--zoomfactor`)
- **Layout**: Left-anchored items grow right; right-anchored items are flex-end
- **z-index**: Above content, below modals (`--zindex-modal-wrapper - 1`)
- **Background**: `rgba(0, 0, 0, 0.35)` — slightly darker than `--main-bg-color`, with a `1px` top border at `--border-color`

---

## Sections (left → right)

### Left Zone

#### 1. Backend Status Indicator
Reflects the health of the `agentmuxsrv-rs` sidecar process.

| State | Icon | Color | Label |
|-------|------|-------|-------|
| Running | `●` filled circle | `--accent-color` (green) | `Backend` |
| Connecting | `◌` spinning | `--warning-color` (yellow) | `Connecting…` |
| Crashed / unreachable | `●` filled circle | `--error-color` (red) | `Backend offline` |

- Derives from the existing heartbeat/sidecar machinery (currently lives in `src-tauri/src/heartbeat.rs` and `sidecar.rs`; expose a Tauri event or command)
- Clicking opens a small popover with the backend process PID, uptime, and log tail

#### 2. Connection Summary
Derives from `allConnStatusAtom` (already in global state).

| State | Display |
|-------|---------|
| No connections | hidden |
| All healthy | `■ {n} connection{s}` in `--secondary-text-color` |
| Any connecting | `◌ {n} connecting` in `--warning-color` |
| Any error | `✕ {n} error` in `--error-color` |

- Shows count, not names (names appear in the block titlebar)
- Clicking opens the existing connection typeahead / connection list modal

### Center Zone (flex spacer — empty by default)

Reserved. Future: active-block breadcrumb (e.g. `~/projects/foo  zsh`), mode indicators.

### Right Zone

#### 3. Config Error Indicator
Moves here from `SystemStatus` in the header.

- Hidden when no errors
- `⚠ Config error` in `--error-color` when `fullConfig.configerrors.length > 0`
- Clicking opens the same MessageModal as the current `ConfigErrorIcon`

#### 4. Update Status
Moves here from `UpdateStatusBanner` in the header.

| `updaterStatus` | Display |
|-----------------|---------|
| `"up-to-date"` | hidden |
| `"downloading"` | `↓ Downloading update…` in `--warning-color` |
| `"ready"` | `↑ Restart to update` in `--accent-color`, clickable |
| `"installing"` | `⟳ Installing…` in `--warning-color` |
| `"error"` | `✕ Update failed` in `--error-color` |

#### 5. Version Label
- `v{version}` — always visible, low-opacity (`0.4`)
- Provides a quick sanity check without competing with the header's new-window button

---

## Component Structure

```
frontend/app/statusbar/
  StatusBar.tsx          ← root component, flex row
  StatusBar.scss
  BackendStatus.tsx      ← left: sidecar health indicator
  ConnectionStatus.tsx   ← left: allConnStatusAtom summary
  UpdateStatus.tsx       ← right: moved from update-banner.tsx
  ConfigStatus.tsx       ← right: moved from system-status.tsx ConfigErrorIcon
```

**StatusBar.tsx** mounts inside `Workspace` (or `AppInner`), below the `PanelGroup`:

```tsx
// workspace.tsx (approximate placement)
<div className="workspace">
    <WindowHeader workspace={workspace} />
    <PanelGroup ...>
        ...
    </PanelGroup>
    <StatusBar />         {/* ← new */}
</div>
```

---

## State / Data Sources

| Item | Existing Source | Notes |
|------|----------------|-------|
| Backend health | New — needs Tauri event/command from `heartbeat.rs` | Emit `backend-status-changed` event |
| Connection count/state | `atoms.allConnStatusAtom` | Already reactive |
| Config errors | `atoms.fullConfigAtom.configerrors` | Already reactive |
| Update status | `atoms.updaterStatusAtom` | Already reactive; move render here |
| Version | `getApi().getAboutModalDetails().version` | Same call as header |

---

## Header Cleanup (what moves out)

Once the status bar lands, remove from `SystemStatus` in the header:
- `UpdateStatusBanner` → replaced by `UpdateStatus` in status bar
- `ConfigErrorIcon` → replaced by `ConfigStatus` in status bar

`ActionWidgets` (help/devtools/custom) stay in the header — they are launch actions, not status.

---

## Sizing & Theme

```scss
.status-bar {
    height: calc(22px * var(--zoomfactor-inv));
    font-size: calc(11px * var(--zoomfactor));
    background: rgba(0, 0, 0, 0.35);
    border-top: 1px solid var(--border-color);
    color: var(--secondary-text-color);
    user-select: none;
    -webkit-app-region: no-drag;
}
```

Interactive items get a hover background of `--hoverbg` and `cursor: pointer`. No buttons — just `div` with `onClick`.

---

## Non-Goals (v1)

- No terminal-mode key hints (tmux-style `^B` legend) — evaluate later
- No per-block status (e.g. exit code, git branch) — belongs in the block titlebar, not the global bar
- No resizing / hiding of the status bar — always visible, fixed height
- No i18n
