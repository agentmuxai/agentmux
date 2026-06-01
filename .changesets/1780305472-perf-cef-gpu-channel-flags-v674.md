---
type: patch
---

perf(cef): enable EarlyEstablishGpuChannel + EstablishGpuChannelAsync

Adds `--enable-features=EarlyEstablishGpuChannel,EstablishGpuChannelAsync`
to the CEF browser-process command line in
`AgentMuxApp::on_before_command_line_processing`. Both features ship
enabled in stable Chrome on Linux and are explicitly set by VSCode's
Electron (confirmed via `/proc/<pid>/cmdline` on a running VSCode); CEF
does not enable them by default.

They (a) request the GPU process channel before the renderer's first
paint instead of synchronously on first paint and (b) treat the channel
establishment as non-blocking, which lets the compositor start producing
frames against the GPU process sooner.

## Empirical impact (Linux Chromium-Ozone-Wayland, 10 panes, 12 s held key)

Measured via `scripts/capture-trace-ipv6.cjs`:

| | Before | After |
|---|---|---|
| `BeginMainFrame` avg | 39.1 ms | **35.7 ms** (-9 %) |
| `BeginMainFrame` count | 12 | 12 (unchanged) |
| Frame cadence | ~1 Hz | ~1 Hz (unchanged) |
| `LayerTreeHostImpl::DidNotProduceFrame` | 66 | 60 |

Small per-frame win (~3 ms / frame). The Wayland frame-production stall
that holds the cadence at ~1 Hz under sysinfo invalidation is a separate
problem (the renderer rejects 60+ Mutter `BeginFrame` requests as
`DidNotProduceFrame` because xterm.js WebGL canvas writes don't surface
as page-level dirtiness). Tracked separately; this PR is just the
matching VSCode flag set so we're not unnecessarily off-default.
