---
type: patch
---

fix(cef): re-inject host IPC creds on every load of our-frontend windows (fixes floating-pane/pool bridge-init lock-out)

on_load_end gated IPC-cred injection on `!is_browser_pane`, which starved floating-pane/pool windows (flagged is_browser_pane but hosting our own frontend). The frontend strips ipc_port/ipc_token from the URL after first read (token-leak fix), so after any reload (Vite HMR, WebGL context-loss reload, bridge auto-recover) those windows arrived cred-less and could never rebuild window.api — the permanent "window.api still undefined after 5s" blank + ~5s reload storm (#52). Now gate injection on frame ORIGIN: inject when the main frame is on our frontend origin (covers floating-pane/pool + main), never for a real remote browser pane (preserves the bearer-token leak protection).
