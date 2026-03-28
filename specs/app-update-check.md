# App Update Check

## Overview

Detect available updates via the GitHub Releases API and show a green "Update Available" indicator in the status bar. Clicking it triggers a platform-specific update flow.

## Motivation

Users on older versions have no way to know an update is available. The status bar already has an `UpdateStatus` component with event infrastructure (`app-update-status`, `UpdaterStatus` type, `install_update` command) — all currently stubbed. This spec fills in the implementation.

## Architecture

```
┌─────────────┐     app-update-status      ┌──────────────┐
│  Backend     │  ──────────────────────►   │  Frontend    │
│  (Rust)      │     event payload          │  StatusBar   │
│              │                            │  UpdateStatus│
│  - periodic  │  ◄──────────────────────   │              │
│    check     │     install_update cmd     │  (click)     │
└─────────────┘                            └──────────────┘
       │
       ▼
  GitHub Releases API
  GET /repos/agentmuxai/agentmux/releases/latest
```

The backend handles all version checking and download logic. The frontend is a thin display layer that reacts to status events.

## Install Type Detection

The app must know how it was installed to determine the correct update path.

### Detection Logic (Rust, at startup)

```
1. Check for MSIX package identity (Windows API: GetCurrentPackageFullName)
   → If present: install_type = "msix"

2. Check if exe is inside a "portable" directory structure
   (exe_dir/bin/agentmuxsrv-rs.x64.exe exists)
   → If present: install_type = "portable"

3. Check for NSIS uninstall registry key
   (HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\AgentMux)
   → If present: install_type = "nsis"

4. Check file extension/path patterns for Linux/macOS:
   - exe path contains ".AppImage" or APPIMAGE env var set → "appimage"
   - exe path under /Applications/ or .app bundle → "dmg"

5. Fallback: "unknown"
```

### Install Types

| Type | Platform | Update Action |
|------|----------|---------------|
| `msix` | Windows | Open Microsoft Store page for AgentMux |
| `nsis` | Windows | Download `.exe` installer, launch it |
| `portable` | Windows | Download `.zip`, open containing folder |
| `dmg` | macOS | Download `.dmg`, open it |
| `appimage` | Linux | Download `.AppImage`, replace in-place |
| `unknown` | Any | Open GitHub releases page in browser |

## Version Check

### Source

```
GET https://api.github.com/repos/agentmuxai/agentmux/releases/latest
Accept: application/vnd.github.v3+json
```

Response (relevant fields):
```json
{
  "tag_name": "v0.33.0",
  "name": "AgentMux v0.33.0",
  "html_url": "https://github.com/agentmuxai/agentmux/releases/tag/v0.33.0",
  "assets": [
    {
      "name": "AgentMux_0.33.0_x64-setup.exe",
      "browser_download_url": "https://github.com/..."
    }
  ]
}
```

### Comparison

Strip the leading `v` from `tag_name` and compare with the running version using semver. If `remote > local`, an update is available.

### Check Schedule

Single check on app startup after a 10 s delay (don't block launch). No periodic re-checks — users who leave the app open for days will see the update next time they restart.

### No Internet

If the request fails (timeout, DNS, HTTP error), silently stay in `up-to-date` state. Never show an error for failed update checks — it's not actionable.

## Status Bar Behavior

### States

The existing `UpdaterStatus` type is reused:

| Status | Indicator | Visible? | Clickable? |
|--------|-----------|----------|------------|
| `up-to-date` | — | No | — |
| `checking` | — | No | — |
| `available` | Green dot + "Update vX.Y.Z" | **Yes** | **Yes** |
| `downloading` | Yellow ↓ + "Downloading..." | Yes | No |
| `ready` | Green ↑ + "Restart to Update" | Yes | Yes |
| `installing` | Yellow ⟳ + "Installing..." | Yes | No |
| `error` | Red ✕ + "Update Failed" | Yes | Yes (retry) |

> **Note:** Add `available` to the `UpdaterStatus` union type. The existing component already handles `downloading`, `ready`, `installing`, `error`.

### Visual Design

The "Update Available" indicator in the status bar:

```
[● Update v0.33.0]
 ^green dot          ^version label
```

- Green background pill/badge style, matching the existing status bar aesthetic
- Positioned in `status-bar-right`, before the version string
- Tooltip: "Click to update AgentMux to v0.33.0"

## Update Flows

### MSIX (Microsoft Store)

1. User clicks "Update v0.33.0"
2. Open the Microsoft Store page for AgentMux:
   ```
   ms-windows-store://pdp/?productid=<STORE_ID>
   ```
   Or fall back to the web URL:
   ```
   https://apps.microsoft.com/detail/<STORE_ID>
   ```
3. Status stays `available` — Store handles the rest
4. No download, no installer — the Store manages everything

### NSIS (Windows Setup Installer)

1. User clicks "Update v0.33.0"
2. Status → `downloading`
3. Backend downloads `AgentMux_X.Y.Z_x64-setup.exe` from the GitHub release asset to a temp directory
4. Status → `ready` with label "Restart to Update"
5. User clicks again → backend launches the installer exe and exits the app
6. The NSIS installer handles the upgrade in-place

### Portable (Windows ZIP)

1. User clicks "Update v0.33.0"
2. Status → `downloading`
3. Backend downloads `AgentMux_X.Y.Z_x64-portable.zip` to the user's Downloads folder
4. Status → `ready` with label "Update Downloaded"
5. User clicks again → open the Downloads folder in Explorer
6. User manually extracts and replaces (portable users expect this)

### macOS (DMG)

1. User clicks "Update v0.33.0"
2. Status → `downloading`
3. Backend downloads `AgentMux_X.Y.Z_aarch64.dmg` to Downloads
4. Status → `ready`
5. User clicks → open the DMG file
6. User drags to /Applications (standard macOS flow)

### Linux (AppImage)

1. User clicks "Update v0.33.0"
2. Status → `downloading`
3. Backend downloads new `.AppImage` to the same directory as the running AppImage
4. Status → `ready` with label "Restart to Update"
5. User clicks → backend replaces the old AppImage with the new one, sets +x, and exits
6. User relaunches

### Unknown / Fallback

1. User clicks "Update v0.33.0"
2. Open `html_url` from the GitHub release in the default browser
3. User downloads manually

## Asset Name Matching

Map install type to GitHub release asset filename pattern:

| Install Type | Asset Pattern |
|-------------|---------------|
| `nsis` | `AgentMux_*_x64-setup.exe` |
| `portable` | `AgentMux_*_x64-portable.zip` |
| `dmg` | `AgentMux_*_aarch64.dmg` or `AgentMux_*_x64.dmg` |
| `appimage` | `AgentMux_*_amd64.AppImage` |
| `msix` | N/A (Store handles it) |

## Implementation

### Files to Modify

| File | Change |
|------|--------|
| `agentmuxsrv-rs/src/backend/updater.rs` | **New** — version check logic, download, install type detection |
| `agentmuxsrv-rs/src/main.rs` | Spawn updater check loop on startup |
| `agentmuxsrv-rs/src/server/` | RPC handler for `install_update` and `check_for_update` |
| `src-tauri/src/commands/stubs.rs` | Remove `install_update` stub, wire to real backend RPC |
| `frontend/types/custom.d.ts` | Add `available` to `UpdaterStatus`, add `UpdateInfo` type |
| `frontend/app/statusbar/UpdateStatus.tsx` | Handle `available` state with green indicator |
| `frontend/util/tauri-api.ts` | Wire up update info (version, release URL) |

### Backend Data Model

```rust
/// Information about an available update.
struct UpdateInfo {
    /// The new version available (e.g., "0.33.0").
    version: String,
    /// URL to the GitHub release page.
    release_url: String,
    /// Direct download URL for the platform-specific asset (if applicable).
    asset_url: Option<String>,
    /// Asset filename for display/logging.
    asset_name: Option<String>,
}

/// How the app was installed — determines the update flow.
enum InstallType {
    Msix,
    Nsis,
    Portable,
    Dmg,
    AppImage,

    Unknown,
}
```

### Frontend Event Payload

Extend the `app-update-status` event payload to include version info:

```typescript
interface UpdateStatusPayload {
    status: UpdaterStatus;        // "available" | "downloading" | "ready" | ...
    version?: string;             // "0.33.0" (when status is "available" or later)
    releaseUrl?: string;          // GitHub release page URL
    downloadProgress?: number;    // 0-100 (when status is "downloading")
}
```

### Pseudocode — Check Loop

```rust
async fn check_for_update_on_startup(event_bus: Arc<EventBus>) {
    // Don't compete with startup
    tokio::time::sleep(Duration::from_secs(10)).await;

    match check_github_release().await {
        Ok(Some(update_info)) => {
            event_bus.emit("app-update-status", json!({
                "status": "available",
                "version": update_info.version,
                "releaseUrl": update_info.release_url,
            }));
        }
        Ok(None) => {} // up-to-date
        Err(_) => {}   // network error — silent
    }
}
```

## Edge Cases

- **Pre-release versions:** Only compare against releases where `prerelease: false` (the `/releases/latest` endpoint already filters these)
- **Rate limiting:** GitHub allows 60 unauthenticated requests/hour. One check per app launch is well within limits
- **Downgrade prevention:** Only show "Update Available" if `remote > local`, not for older versions
- **Multiple instances:** Each instance checks independently — one request per launch is negligible
- **Offline mode:** No update check, no error shown, no impact on app functionality

## Testing

1. Mock the GitHub API response with a newer version → verify green indicator appears
2. Mock with same version → verify no indicator
3. Mock with network error → verify no error shown
4. Click update on each install type → verify correct action
5. Verify 10s startup delay before first check
