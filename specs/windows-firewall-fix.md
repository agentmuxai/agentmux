# Windows Firewall Popup — Root Cause & Fix

**Status:** Root cause identified, fix is trivial
**Impact:** Annoying UX — user can click Cancel and app works fine

---

## Root Cause

The firewall popup is NOT triggered by the localhost IPC servers. Both
`agentmux-cef.exe` and `agentmuxsrv-rs.exe` bind to `127.0.0.1:0` (loopback),
which Windows Firewall **does not intercept**. This is confirmed by Chrome,
which does the same for DevTools on port 9222 without a prompt.

The trigger is **mDNS LAN discovery**. The `mdns_sd::ServiceDaemon` in
`agentmuxsrv-rs/src/backend/lan_discovery.rs` binds **UDP port 5353 on 0.0.0.0**
(all interfaces) for multicast DNS. Binding to `0.0.0.0` is a network-facing
operation that Windows Firewall intercepts.

A secondary trigger may be CEF's WebRTC mDNS candidate generation, which also
binds UDP 5353 on `0.0.0.0`.

## Evidence

| Component | Bind Address | Triggers Firewall? |
|-----------|-------------|-------------------|
| CEF IPC server (`ipc.rs:94`) | `127.0.0.1:0` TCP | No |
| Backend web server (`main.rs:384`) | `127.0.0.1:0` TCP | No |
| Backend WebSocket (`main.rs:387`) | `127.0.0.1:0` TCP | No |
| **mDNS daemon** (`lan_discovery.rs:53`) | **`0.0.0.0:5353` UDP** | **YES** |
| CEF WebRTC mDNS | `0.0.0.0:5353` UDP | Possibly |

The code already handles mDNS failure gracefully (line 411: "LAN discovery
unavailable"), which is why clicking Cancel works fine.

## Fix — Phase 1 (immediate, solves the problem)

### 1. Disable mDNS LAN discovery by default

The `LanDiscovery::start()` call creates a `ServiceDaemon` that binds
`0.0.0.0:5353`. Make it opt-in via settings:

```rust
// In agentmuxsrv-rs startup, check settings before starting LAN discovery:
if settings.lan_discovery_enabled.unwrap_or(false) {
    LanDiscovery::start(...);
} else {
    tracing::info!("LAN discovery disabled (enable in settings to discover other instances)");
}
```

Most users don't need multi-instance LAN discovery. The feature can be
enabled later from settings without restarting.

### 2. Disable CEF WebRTC mDNS (if CEF < v130)

Add to CEF command-line initialization:

```rust
// In app.rs or main.rs CEF initialization:
command_line.append_switch_with_value(
    "disable-features",
    "WebRtcHideLocalIpsWithMdns"
);
```

AgentMux doesn't use WebRTC, so this is safe. Note: this flag was
removed in Chromium ~v130, but may still apply to CEF 146 depending
on build configuration. For newer CEF, use:
`--force-webrtc-ip-handling-policy=default_public_interface_only`

## Fix — Phase 2 (if firewall persists after Phase 1)

### 3. Verify with netstat

After applying Phase 1, run the app and check:
```cmd
netstat -ano | findstr agentmux
```
All entries should show `127.0.0.1:*` — no `0.0.0.0:*` bindings.

### 4. Named Pipes (future hardening)

Replace TCP IPC with Windows named pipes to eliminate TCP entirely:
- `tokio::net::windows::named_pipe` is fully supported
- Axum can serve over named pipes via raw hyper
- Named pipes never touch the network stack
- Pattern: `\\.\pipe\agentmux-ipc-{pid}`
- Tradeoff: more code, harder to debug (can't curl a named pipe)

## Why Other Approaches Don't Work

| Approach | Why Not |
|----------|---------|
| Programmatic firewall rules | Requires admin elevation — bad for portable apps |
| Code signing | Improves dialog text but doesn't suppress it |
| Windows manifest | No manifest element suppresses the firewall prompt |
| Firewall rules in installer | Only works for installed apps, not portable |

## How Tauri Avoids It

Tauri uses custom protocols (`tauri://`) via WebView2's
`AddWebResourceRequestedFilter` — IPC never touches the network stack.
No TCP listener, no firewall concern.

## How Chrome Handles It

Chrome binds DevTools to `127.0.0.1:9222` (no firewall prompt).
The Chrome firewall popup that users sometimes see is from mDNS
(port 5353 on 0.0.0.0) for Cast discovery and WebRTC ICE candidates —
exactly the same root cause as ours.

---

## Summary

Disable mDNS LAN discovery by default → firewall popup gone.
One config flag, zero risk, the feature already handles failure gracefully.
