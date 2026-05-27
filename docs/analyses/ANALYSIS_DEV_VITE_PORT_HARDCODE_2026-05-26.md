# Analysis: Dev Vite port hardcoded in `resolve_frontend_base_url`

**Date:** 2026-05-26
**Status:** Fix landing in same PR

## TL;DR

`agentmux-cef/src/commands/window.rs:619` hardcodes `http://localhost:5173`
for dev mode, ignoring the `AGENTMUX_VITE_PORT` env var that `task dev`
sets when the auto-derived per-clone port differs from 5173. The **main
window** survives because the launcher passes `--url=$VITE_URL` on the
CLI, but **every child window** — window-pool warmups, tab tear-off
windows, and the new floating pane window — calls
`resolve_frontend_base_url` and loads `localhost:5173`, hitting
`ERR_CONNECTION_REFUSED` whenever Vite is on any other port.

Surfaced during smoke-testing of #1078 (floating-pane Phase 2) — the
test session used `AGENTMUX_VITE_PORT=5350` to dodge a collision on the
auto-derived 5270, and both tab tear-off and pane tear-off failed to
load their new browsers.

This is **not a regression from #1078** — it has been latent in every
dev session whose auto-derived Vite port ≠ 5173. The earlier multi-clone
analysis ([ANALYSIS_MULTI_CLONE_TASK_DEV_ISOLATION_2026-05-26.md][])
flagged port 5173 as a "Low severity / loud collision" — wrong: when
the script *successfully* moves the port off 5173, child windows fail
silently with cryptic `ERR_CONNECTION_REFUSED` errors and no visible
explanation in the UI.

## Evidence (dev session 2026-05-26 19:31 PT, Vite on 5350)

Child windows attempted to load 5173:

```
02:31:16.997  Load error: url=http://localhost:5173/?...&pool=1
              error=ERR_CONNECTION_REFUSED (-102)              # pool warmup
02:31:29.839  Load error: url=http://localhost:5173/?...&floatingPaneId=…
              error=ERR_CONNECTION_REFUSED (-102)              # pane tear-off
02:31:36.946  Load error: url=http://localhost:5173/?...&workspaceId=…
              error=ERR_CONNECTION_REFUSED (-102)              # tab tear-off
```

Main window worked because the launcher invocation passes the resolved
port on the CLI:

```bash
# Taskfile.yml dev:serve, after auto-deriving AGENTMUX_VITE_PORT
./agentmux-launcher.exe --url="$VITE_URL"   # VITE_URL=http://localhost:5350
```

## Call graph

Every dev child-window URL goes through one function:

| Caller                                              | Purpose                       |
|-----------------------------------------------------|-------------------------------|
| `agentmux-cef/src/commands/window_pool.rs:196`      | Pool browser warmup           |
| `agentmux-cef/src/commands/window.rs:712`           | New top-level window          |
| `agentmux-cef/src/commands/drag.rs:431`             | Tab tear-off                  |
| `agentmux-cef/src/floating_pane.rs:209`             | Floating pane (Phase 1+)      |
| `agentmux-cef/src/client/mod.rs:1280`               | Misc child reloads            |

All of them call `resolve_frontend_base_url(ipc_port)` in
`agentmux-cef/src/commands/window.rs:604`, whose dev branch returns:

```rust
if matches!(mode, Some(agentmux_common::RuntimeMode::Dev { .. })) {
    return "http://localhost:5173".to_string();
}
```

## Why the main window dodges the bug

`dist/cef-dev/` is launched as:

```bash
AGENTMUX_DEV=1 ./agentmux-launcher.exe --url="$VITE_URL"
```

The host treats `--url=…` as authoritative for the first browser it
creates (the main window). `resolve_frontend_base_url` is only consulted
for *subsequent* browsers, where the CLI override no longer applies.

## Fix

Make `resolve_frontend_base_url` honor `AGENTMUX_VITE_PORT`. The env var
is already exported by `Taskfile.yml`'s `dev:serve` task and inherited
transitively by the host (launcher → host via tokio::process::Command::spawn,
which inherits the full env block).

```rust
fn dev_vite_port() -> u16 {
    std::env::var("AGENTMUX_VITE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5173)
}
```

Apply at both dev-mode branches (the explicit dev path *and* the no-frontend
production fallback that also returns `localhost:5173`).

~6 lines. Zero risk: identical behavior in the common case (port = 5173);
correct behavior in the per-clone-derived case.

## What this doesn't fix

- **Pool-exhaustion warning on tear-off** (`[pool] pool exhausted on tear-off
  — frontend will cold-path`) — orthogonal; cold-path is supposed to work and
  would have worked if the URL had been correct.
- **`[on_before_close] no backend window ID registered for label=…floating…
  — shells may orphan`** — backend window-id-registry plumbing for floating
  panes is a separate gap, tracked alongside #1079.

## Cross-references

- Issue #1079 — floating-pane C1/C2 (macOS NSPanel + Linux X11 GTK)
- PR #1078 — Phase 2 (real `<Block>` renderer), shipped 2026-05-26
- [ANALYSIS_MULTI_CLONE_TASK_DEV_ISOLATION_2026-05-26.md][] — earlier
  analysis that under-rated this collision as "Low severity / loud" when
  in fact silent child-window failures are the real consequence.
- `Taskfile.yml` `dev:serve` task — derives `AGENTMUX_VITE_PORT` per-clone
  via cksum-of-cwd modulo 200.

[ANALYSIS_MULTI_CLONE_TASK_DEV_ISOLATION_2026-05-26.md]: ./ANALYSIS_MULTI_CLONE_TASK_DEV_ISOLATION_2026-05-26.md
