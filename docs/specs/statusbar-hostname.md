# Spec: Hostname Display in Status Bar

**Date:** 2026-03-26
**Author:** agent2
**Status:** Draft

## Goal

Display the system hostname in the status bar, aligned right near the version indicator. This surfaces the "host" concept in the AgentMux UI, helping users identify which machine the app is running on — especially useful with remote/SSH connections.

## Current State

- **Status bar right section** contains: `ConnectionStatus`, `ConfigStatus`, `UpdateStatus`, version display
- **`getApi().getHostName()`** already exists — returns the system hostname via Tauri's `get_host_name` command (uses `whoami::fallible::hostname()` in Rust)
- The hostname is fetched at app startup alongside platform, userName, etc. in `tauri-api.ts` and cached

## Changes

### 1. `frontend/app/statusbar/StatusBar.tsx`

Add hostname display to the right section, immediately before the version span:

```tsx
const hostname = getApi().getHostName();

// In the JSX, before the version <Show>:
<Show when={hostname}>
    <span class="status-hostname" title={`Host: ${hostname}`}>
        {hostname}
    </span>
</Show>
```

The hostname appears as a static text label (not clickable). It sits to the left of the version, separated by the standard status bar gap.

### 2. `frontend/app/statusbar/StatusBar.scss`

Add styles for the hostname label:

```scss
.status-hostname {
    font-size: 11px;
    color: var(--status-bar-text-muted);
    user-select: text;       // Allow copying hostname
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 200px;        // Truncate very long hostnames
}
```

## Layout (right section, left to right)

```
[ConnectionStatus?] [ConfigStatus?] [UpdateStatus?] [hostname] [v1.2.3 (2)]
```

The `?` items are conditional — they only render when relevant:
- **ConnectionStatus** — visible only when remote connections (SSH/WSL) exist
- **ConfigStatus** — visible only when there are config errors
- **UpdateStatus** — visible only during update download/install/error

In typical usage the right section shows just: `hostname` + `version`.

## Edge Cases

- **Long hostnames (FQDNs):** Truncated with ellipsis at 200px; full name shown in `title` tooltip
- **Hostname unavailable:** `getHostName()` returns `"unknown"` on failure — show nothing (`<Show when={hostname && hostname !== "unknown"}>`)
- **No backend changes needed** — `get_host_name` command already exists and is cached at startup

## Files Modified

| File | Change |
|------|--------|
| `frontend/app/statusbar/StatusBar.tsx` | Add hostname display element |
| `frontend/app/statusbar/StatusBar.scss` | Add `.status-hostname` styles |

## Not in Scope

- Remote host detection (SSH hostname) — that's a separate feature
- Editable/configurable hostname label
- Click behavior on hostname
