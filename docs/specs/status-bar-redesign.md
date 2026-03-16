# Spec: Status Bar Redesign

**Goal:** Replace the current sparse status bar with a dense, useful information display. Remove confusing "connection count" metric and add system/session stats that are actually valuable.

**Status:** Ready for implementation.

---

## Current State

The status bar (22px, bottom of window) currently shows:

| Left | Center | Right |
|------|--------|-------|
| Backend status (● Running) | *(empty spacer)* | Config errors (if any) |
| Connection count (e.g. "1 connection") | | Update status (if pending) |
| | | Version (e.g. v0.32.5) |

**Problems:**
1. **"1 connection" is confusing** — implies other frontends are connected to the backend. In practice it's always 1 (this window's websocket). Only meaningful for SSH/WSL remote connections, which are niche.
2. **Wasted space** — the center section is completely empty. The status bar is prime real estate for at-a-glance system info.
3. **Backend status is redundant** — it's almost always "Running". The dot is useful but the word takes space.

---

## Proposed Layout

```
[● Backend 0:42:13] [CPU 12% | Mem 3.2G/16G] [↑1.2M ↓340K]   [⚠ Config] [↑ Update] v0.32.5
|---- left --------------------------------------------|          |-------- right --------|
```

### Left Section (system stats)

| Item | Display | Source | Click Action |
|------|---------|--------|-------------|
| **Backend uptime** | `● 2h 13m` | `getApi().getBackendInfo().started_at` | Popover with PID, endpoint, version (existing) |

**Live ticking:** Uptime updates every 1 second via a client-side `setInterval`. The `started_at` timestamp is fetched once on mount — the timer is pure `Date.now() - startedAt` math, zero backend overhead.

**Uptime format:** Always show the two largest non-zero denominations:

| Duration | Display |
|----------|---------|
| < 1 minute | `42s` |
| < 1 hour | `12m 5s` |
| < 1 day | `5h 23m` |
| < 1 week | `4d 7h` |
| < 1 month | `2w 3d` |
| < 1 year | `4mo 12d` |
| 1+ years | `2yr 7mo` |
| 5+ years | `5yr 4mo` |
| **System CPU** | `CPU 12%` | `sysinfo` WPS events (already collected) | None |
| **System Memory** | `Mem 3.2G/16G` | `sysinfo` WPS events (already collected) | None |
| **Network I/O** | `↑1.2M ↓340K` | `sysinfo` WPS events (if available) | None |

### Center Section (session info)

| Item | Display | Source | Click Action |
|------|---------|--------|-------------|
| **Active panes** | `4 panes` | `leafCount` from layout model | None |
| **Focused connection** | `ssh:devbox` | Focused block's connection meta | Click to open conn modal |

The center section is optional / lower priority. Can ship without it.

### Right Section (unchanged, minus connection count)

| Item | Display | Source | Click Action |
|------|---------|--------|-------------|
| **Config errors** | `⚠ Config` | `fullConfigAtom.configerrors` | Modal (existing) |
| **Update status** | `↑ Update` | `updaterStatusAtom` | Install update (existing) |
| **Remote connections** | `2 remote` | `allConnStatus` filtered to non-local | Modal listing remotes (existing modal) |
| **Version** | `v0.32.5` | Package version | Open new window (existing) |

**Key change:** Replace generic "X connections" with "X remote" that only shows when there are actual SSH/WSL connections. Hidden when 0 remotes.

---

## Implementation

### 1. New component: `SystemStats.tsx`

Subscribe to the existing `sysinfo` WPS events (same data the sysinfo view uses). Display CPU and memory as compact inline badges.

```typescript
// Already published by backend every 5s:
// eventType: "sysinfo", data: { cpu, memory: { total, used, free }, ... }
```

No new backend work needed — the sysinfo collector already publishes global system metrics via WPS.

### 2. Modify `BackendStatus.tsx`

- Remove the word "Running" / "Backend" from inline display
- Show uptime inline: `● 2h 13m` (green dot + compact duration)
- Keep the popover with full details on click

### 3. Modify `ConnectionStatus.tsx`

- Filter out the local websocket connection (always 1, not useful)
- Only show when `remoteConnections.length > 0`
- Label: `"X remote"` instead of `"X connections"`

### 4. New component: `NetworkStats.tsx` (optional, phase 2)

If sysinfo publishes network I/O rates, show `↑1.2M ↓340K`. Skip if not available in current sysinfo data.

### 5. Layout change in `StatusBar.tsx`

```tsx
<div class="status-bar">
    <div class="status-bar-left">
        <BackendStatus />       {/* ● 2h 13m */}
        <SystemStats />         {/* CPU 12% | Mem 3.2G/16G */}
    </div>
    <div class="status-bar-center">
        {/* future: pane count, focused connection */}
    </div>
    <div class="status-bar-right">
        <ConnectionStatus />    {/* only remote connections */}
        <ConfigStatus />
        <UpdateStatus />
        <VersionDisplay />
    </div>
</div>
```

---

## Data Sources

| Metric | Already Available? | Source |
|--------|-------------------|--------|
| Backend PID, uptime, endpoint | Yes | `getApi().getBackendInfo()` |
| CPU % (global) | Yes | `sysinfo` WPS event |
| Memory used/total | Yes | `sysinfo` WPS event |
| Network I/O | Check | May need backend addition |
| Remote connection count | Yes | `allConnStatus` atom, filter `conn !== ""` |
| Pane count | Yes | Layout model `leafCount` |
| Focused connection | Yes | Focused block's `connection` meta |

---

## Styling

- Keep 22px height, 11px font
- Stats use `var(--secondary-text-color)` at 60% opacity, brighten on hover
- CPU/Mem numbers use monospace font for stable width
- Separator between stat groups: thin `|` at 20% opacity
- CPU color thresholds: >80% amber, >95% red (matching per-pane badge)
- Memory color: >90% used → amber

---

## Files Changed

| File | Change |
|------|--------|
| `frontend/app/statusbar/SystemStats.tsx` | **New** — CPU + memory display |
| `frontend/app/statusbar/BackendStatus.tsx` | Compact to dot + uptime |
| `frontend/app/statusbar/ConnectionStatus.tsx` | Filter to remote-only, rename label |
| `frontend/app/statusbar/StatusBar.tsx` | Layout restructure |
| `frontend/app/statusbar/StatusBar.scss` | Styles for new items |

---

## Testing

1. Status bar shows CPU % and memory — updates every 5s
2. Backend uptime ticks correctly, popover still works
3. Connection count hidden when only local connection exists
4. Connection count shows "2 remote" when SSH connections active
5. Chrome zoom still affects status bar correctly
6. No layout shift when stats update (monospace numbers, fixed widths)
