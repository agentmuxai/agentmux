# AGENTMUX_LOCAL_URL Pane Injection

**Status:** Approved
**Date:** 2026-03-16
**Author:** AgentY

---

## Problem

`inject_terminal` (jekt) times out for host agents.

The `agentbus-client` MCP server has two delivery paths:

1. **Local** — `POST {AGENTMUX_LOCAL_URL}/wave/reactive/inject` → AgentMux writes directly to the PTY. Sub-millisecond, no cloud involved.
2. **Cloud polling** — polls `agentbus.asaf.cc/reactive/pending/{agentId}` every 5s, delivers via `console.error()` on the MCP server's stderr. Stderr from an MCP child process is not fed back as user input in Claude Code, so injections are silently lost.

Path 1 is the correct path. Path 2 doesn't work for in-pane injection.

`AGENTMUX_LOCAL_URL` is only active when it's present in the MCP server's environment. Since each agent's MCP config is static (`.mcp.json`), we can't hardcode a fixed port — `agentmuxsrv-rs` binds to `127.0.0.1:0` (OS-assigned port) on every startup.

The fix: **AgentMux injects `AGENTMUX_LOCAL_URL` into each pane's environment at creation time.** Since AgentMux knows its own backend port, each pane automatically gets the correct URL for the instance that owns it.

---

## Why Not a Fixed Port

- Two instances running simultaneously (e.g. different versions) would conflict.
- Port 0 binding is intentional — avoids "port already in use" failures.
- A fixed port would require user configuration and break multi-version installs.

---

## Solution

Inject `AGENTMUX_LOCAL_URL=http://127.0.0.1:{web_port}` into every shell pane's environment, alongside the existing `AGENTMUX_AGENT_ID`, `AGENTMUX_VERSION`, etc.

**Multi-instance correctness:** each AgentMux instance injects its own backend port. A pane owned by instance A on port 58341 points to `http://127.0.0.1:58341`. A pane owned by instance B on port 62288 points to `http://127.0.0.1:62288`. No conflicts.

**agentbus-client behavior** (already implemented in v1.0.15):

```javascript
// client.js getConfigFromEnv()
localUrl: process.env.AGENTMUX_LOCAL_URL || process.env.WAVEMUX_LOCAL_URL

// index.js startup
if (config.localUrl) {
    // Path A: local delivery, cloud polling disabled
    console.error(`[AgentBus MCP] Local delivery: ${config.localUrl} (cloud polling disabled)`);
} else if (config.token) {
    // Path B: cloud polling (fallback only)
    stopPolling = startInjectionPolling(config, 5000);
}
```

When `AGENTMUX_LOCAL_URL` is set, the client routes all `inject_terminal` calls to the local AgentMux backend. No cloud round-trip, no polling, no timeout.

---

## Implementation

### Approach: `std::env::set_var` + Process Env Inheritance

`main.rs` sets `AGENTMUX_LOCAL_URL` as a process-level env var immediately after binding the web listener. Child processes (PTY shells) inherit the parent process's environment, so every pane automatically gets the correct URL. `shell.rs` re-injects it explicitly via `c.env(...)` to ensure it survives any env filtering in portable-pty.

**`main.rs`** — after binding `web_listener`, before `WAVESRV-ESTART`:

```rust
let web_addr = web_listener.local_addr().unwrap();
let local_web_url = format!("http://{}", web_addr);

// Make local backend URL available to child processes (PTY shells).
// agentbus-client reads AGENTMUX_LOCAL_URL and uses it for local PTY delivery
// instead of routing through the cloud agentbus.
std::env::set_var("AGENTMUX_LOCAL_URL", &local_web_url);
```

**`shell.rs`** — in the env var injection block, after existing `AGENTMUX_*` vars:

```rust
// Propagate local backend URL so agentbus-client prefers local PTY delivery.
// Set by main.rs after binding; absent in test/mock contexts (graceful no-op).
if let Ok(local_url) = std::env::var("AGENTMUX_LOCAL_URL") {
    c.env("AGENTMUX_LOCAL_URL", &local_url);
}
```

---

## Affected Files

| File | Change |
|------|--------|
| `agentmuxsrv-rs/src/main.rs` | `std::env::set_var("AGENTMUX_LOCAL_URL", ...)` after binding + store in `AppState.local_web_url` |
| `agentmuxsrv-rs/src/backend/blockcontroller/shell.rs` | Re-inject `AGENTMUX_LOCAL_URL` into PTY env via `std::env::var` |

---

## No Changes Required

- `agentbus-client` already handles `AGENTMUX_LOCAL_URL` (v1.0.15+)
- `.mcp.json` templates don't need updating — env var comes from the pane
- Container agents (Agent1-5) are unaffected — they connect via cloud path or host networking

---

## Testing

After building and running `task dev`:

1. Open a new pane — verify `echo $AGENTMUX_LOCAL_URL` shows `http://127.0.0.1:{port}`
2. From an agent pane, call `inject_terminal` targeting itself — should return `status: delivered` within 1-2s
3. Start a second AgentMux instance — verify each instance's panes show different ports
