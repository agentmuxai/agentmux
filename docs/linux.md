# Linux — operator guide

## AppImage structure

The AppImage entry point is the **launcher** (since v0.42.x, A0 / PR #1286). When you run the AppImage:

1. `AppRun` → `usr/bin/linux-apprun.sh`
2. On first launch, the script extracts itself to `~/.local/share/agentmux/extracted/<version>/` (~2–3 s one-time). Subsequent launches re-exec from the cache (~1 s).
3. `linux-apprun.sh` execs `usr/bin/agentmux-launcher`
4. The launcher spawns `agentmux-srv` (backend) and `agentmux-cef` (CEF host) as a supervised process group via `PR_SET_PDEATHSIG`
5. The launcher binds a Unix-socket IPC server; the host connects and reports window lifecycle events
6. The host opens the main window and loads the frontend from the embedded `usr/bin/frontend/`

**AppImage contents:**

```
usr/bin/
├── agentmux-launcher              # AppRun entry point — supervises srv + host
├── agentmux-cef                   # CEF host binary
├── agentmux-srv-{version}-linux.x64  # Rust async backend
├── libcef.so                      # Chromium runtime (~613 MB stripped)
├── libEGL.so / libGLESv2.so       # GPU abstraction
├── *.pak / icudtl.dat / …         # Chromium resources
└── frontend/                      # Bundled web UI
```

## Display server

By default, AgentMux uses **XWayland** (`--ozone-platform=x11`) under any Wayland compositor (Mutter, KWin, etc.). This default provides the best frame-rate consistency across GPU configurations (5–8× fewer stalls than native Wayland on the tested hardware/driver set).

Set `AGENTMUX_OZONE_PLATFORM=wayland` to use native Wayland (`xdg_toplevel`). This is experimental.

## Window drag

Title-bar drag and floating-pane header drag use `CefWindow::BeginWindowDrag()` — a native AgentMux patch that dispatches `xdg_toplevel.move` (Wayland) or `_NET_WM_MOVERESIZE` (X11/XWayland). The patched `libcef.so` must be present (all release AppImages include it).

Dev builds compile with `--features patched-libcef` automatically via `task build:host:linux`.

See [`docs/cef-build/build-patched-libcef.md`](cef-build/build-patched-libcef.md) for building libcef from source.

## Log access

```bash
muxlog host                    # tail the CEF host log ([fe] lines = frontend)
muxlog srv                     # tail the backend sidecar log
muxlog host '\[fe\]'           # frontend-only lines
muxlog host cat                # full host log (not tailed)
```

The launcher writes an append-only JSONL event log to `~/.agentmux/channels/<ch>/versions/<v>/data/launcher-events.log` (crash forensics — one JSON object per line). The saga journal is a separate SQLite database at `~/.agentmux/channels/<ch>/versions/<v>/data/db/launcher-sagas.db` — that is what `--diag sagas` reads.

`$AGENTMUX_LOG_DIR` inside AgentMux terminals points to `~/.agentmux/logs/` (stable channel) or `~/.agentmux-dev/logs/` (dev builds). The host log lives in the per-instance data dir; a pointer file (`current-host-v<v>.path`) in the shared logs dir lets `muxlog host` resolve it automatically.

## Launcher diagnostics

The launcher runs the full reducer + saga coordinator on Linux (since v0.42.x A1 / PR #1288 — same as Windows). Offline diagnostic commands (no running instance required):

```bash
./AgentMux_*.AppImage --diag sagas   # dump the saga journal (SQLite log of lifecycle events)
```

:::note[Windows-only diagnostics]
`--diag wrr` (window/pool/reducer state) and `--diag srv` (live sidecar query) are Windows-only today — the Unix-domain-socket IPC client for these tools is not yet implemented. See [Platform support](/internals/platform-support/).
:::

## Remote debugging

The CEF host starts a remote debugger on port 9222 (release builds) or 9223 (dev builds). Connect from a Chromium-based browser:

1. Start AgentMux
2. Open `chrome://inspect` in another Chromium browser
3. Under "Remote Target", click "Configure…" and add `localhost:9222` (release builds) or `localhost:9223` (dev builds via `task dev`)
4. The AgentMux renderer process appears under "Remote Target"

## Single-instance enforcement

The launcher enforces single-instance per `(data_dir, version)` pair via a Unix domain socket. Socket location:

- Primary: `$XDG_RUNTIME_DIR/agentmux/{hash16}.sock`
- Fallback (no `$XDG_RUNTIME_DIR`): `/tmp/agentmux-{uid}/{hash16}.sock`

Opening a second instance sends an `open_new_window` command to the running launcher and exits immediately.

## Known limitations (as of v0.43.x)

| Feature | Status |
|---|---|
| Splash screen | Not yet implemented (Windows + macOS have native splash screens) |
| Window transparency | Under investigation — root cause identified (views::SolidBackground), fix blocked on Mutter wl_surface visibility without opaque base pixel |
| Native Wayland (non-XWayland) | Experimental; set `AGENTMUX_OZONE_PLATFORM=wayland` |
| Linux .deb package | Produced by CI builder (`agentmuxai/agentmux-builder`) only, not by `task package:linux` |
| Owned-window floaters (`transient-for` + destroy-with-parent) | Phase B, not yet implemented — floaters open as independent top-level windows |
