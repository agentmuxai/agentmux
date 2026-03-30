# Hostname Popover — Network & Instance Info

**Status:** Spec
**Location:** Status bar, right side — click the hostname text

---

## Current State

The hostname is a plain `<span>` in the status bar right side with no interactivity.
LAN discovery (mDNS) starts automatically on launch, triggering the Windows Firewall
prompt because `mdns_sd::ServiceDaemon` binds `0.0.0.0:5353` UDP.

```
Status bar (current):
[● 12:34 | CPU 15% | Mem 4.2G/16G | ↑0K ↓0K]  [◆ 2 on LAN] [conn] [⚙] [Area54] [v0.32.112]
                                                                        ^^^^^^^^
                                                                        plain text, no click
```

## Goal

Make the hostname clickable. Opens a popover with network info, instance details,
and an mDNS toggle. mDNS starts **off by default** (no firewall prompt on first launch).
User enables it from the popover when they want LAN discovery.

```
Status bar (new):
[● 12:34 | CPU 15% | Mem 4.2G/16G]  [conn] [⚙] [Area54 ▾] [v0.32.112]
                                                  ^^^^^^^^^
                                                  clickable, opens popover
```

## Popover Content

```
┌─────────────────────────────────┐
│  Area54                         │  ← hostname (bold)
│  Windows 10 Pro x64             │  ← OS
│  192.168.1.42                   │  ← local IP (primary interface)
│─────────────────────────────────│
│  Instance                       │
│  ID       v0.32.112             │
│  Data     ~\AppData\...\v0-32-112│
│  Host     CEF 146 / Chromium    │  ← or "Tauri / WebView2"
│  PID      12345                 │  ← host process PID
│─────────────────────────────────│
│  Network                        │
│  ◌ LAN Discovery    [Enable]   │  ← toggle button
│    Broadcasts this instance     │
│    on the local network via     │
│    mDNS (port 5353)             │
│                                 │
│  (when enabled and peers found):│
│  ◆ 2 instances on LAN          │  ← moves here from status bar
│    Workstation2 v0.32.112       │
│    Laptop-Dev v0.32.111         │
│─────────────────────────────────│
│  Ports                          │
│  IPC      127.0.0.1:54595      │
│  Backend  127.0.0.1:54596      │
│  WS       127.0.0.1:54597      │
│  DevTools 127.0.0.1:9222       │
└─────────────────────────────────┘
```

## Sections

### 1. Host Identity

| Field | Source |
|-------|--------|
| Hostname | `getApi().getHostName()` (already in StatusBar) |
| OS | `getApi().getPlatform()` + version |
| Local IP | New — backend resolves primary non-loopback IPv4 |

### 2. Instance Info

| Field | Source |
|-------|--------|
| Instance ID | Version string (v0.32.112) |
| Data dir | `getApi().getDataDir()` |
| Host type | CEF or Tauri (detect from `window.__AGENTMUX_IPC_PORT`) |
| PID | `getApi().getBackendInfo().pid` (already fetched) |

### 3. Network / LAN Discovery

| Field | Source |
|-------|--------|
| mDNS toggle | New setting: `lan_discovery_enabled` (default: false) |
| LAN instances | Existing `lanInstancesAtom` (move from separate LanStatus widget) |

**Toggle behavior:**
- Default: OFF (mDNS not started, no firewall prompt)
- Click "Enable" → backend starts mDNS daemon, button changes to "Disable"
- Persisted in settings.json: `"network:lan_discovery": true`
- On next launch, if setting is true, mDNS starts automatically (user already accepted firewall)

### 4. Ports

| Field | Source |
|-------|--------|
| IPC port | `state.ipc_port` |
| Backend web | `state.backend_endpoints.web_endpoint` |
| Backend WS | `state.backend_endpoints.ws_endpoint` |
| DevTools | Hardcoded 9222 (CEF only) |

---

## Implementation

### Frontend Changes

**New file:** `frontend/app/statusbar/HostPopover.tsx`
- Popover component (same pattern as BackendStatus popover)
- Click hostname → toggle popover
- Outside click → close

**Modified:** `frontend/app/statusbar/StatusBar.tsx`
- Replace plain hostname `<span>` with `<HostPopover>`
- Remove `<LanStatus />` from status bar left (absorbed into popover)

### Backend Changes

**New IPC commands:**

| Command | Returns |
|---------|---------|
| `get_local_ip` | Primary non-loopback IPv4 address string |
| `get_host_info` | `{ hostname, os, local_ip, instance_id, data_dir, host_type, pid, ports: { ipc, web, ws, devtools } }` |
| `enable_lan_discovery` | Starts mDNS daemon, returns success/error |
| `disable_lan_discovery` | Stops mDNS daemon |
| `get_lan_discovery_status` | `{ enabled: bool, instance_count: number }` |

**Modified:** `agentmuxsrv-rs` startup
- Do NOT start mDNS by default
- Check `settings.json` for `"network:lan_discovery": true`
- Only start mDNS if explicitly enabled

### Settings Schema

```jsonc
{
  // ... existing settings ...
  "network:lan_discovery": false  // default off, no firewall prompt
}
```

---

## Firewall Fix (the original motivation)

By making mDNS opt-in:
- First launch: no `0.0.0.0:5353` bind → no firewall prompt
- User enables LAN discovery from hostname popover → accepts firewall prompt once
- Setting persists → subsequent launches start mDNS automatically
- User who never enables it never sees the firewall prompt

---

## Migration from Current LanStatus

The existing `<LanStatus />` component in the status bar shows "◆ N on LAN"
when mDNS finds peers. This moves into the hostname popover's Network section.
The status bar becomes cleaner (no separate LAN indicator).

If mDNS is enabled and peers are found, the hostname text could show a subtle
indicator: `Area54 ◆` (diamond dot after hostname) to signal LAN peers exist
without taking up a separate status bar slot.

---

## Visual Design

- Same popover style as BackendStatus (dark background, monospace values)
- Hostname in bold at top
- Sections separated by `<div class="status-bar-popover-divider">`
- Toggle button styled like `status-bar-restart-btn`
- Ports in monospace
- LAN instances list same style as existing LanInstancesModal
