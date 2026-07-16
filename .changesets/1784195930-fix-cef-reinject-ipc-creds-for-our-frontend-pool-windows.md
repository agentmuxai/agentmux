---
type: patch
---

fix(cef): recover host bridge on reload for floating-pane/pool windows (#52)

Two-part fix for the "window.api still undefined after 5s" blank + ~5s reload storm on floating-pool windows. (1) Host: on_load_end gated IPC-cred injection on `!is_browser_pane`, starving floating-pane/pool windows (flagged is_browser_pane but hosting our own frontend). Now gate on frame ORIGIN so those windows get creds re-injected on every load, while a real remote browser pane still never receives the bearer token. (2) Frontend: setupCefApi strips ipc_port/ipc_token from the URL after first read, so on reload isCef() saw no creds and bailed before awaiting the re-injection. isCef() now remembers "we are CEF" in sessionStorage (set on first ipc_port sighting), surviving the strip and any reload so setupCefApi reaches waitForIpcCreds and picks up the host re-injection.
