---
type: patch
---

A headless server can show a Browser pane in its web UI: a Chromium on the server streams the page as JPEG frames over a WebSocket and takes mouse, wheel, keyboard and paste back (off unless the server is started with AGENTMUX_STREAMED_BROWSER=1; this slice is the server side and a probe, the pane in the web UI follows)
