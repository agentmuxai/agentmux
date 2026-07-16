---
type: patch
---

fix(cef): closing the last window ("main") never notified srv, leaking its rows forever

Closing "main" (the app's primary/last window) was structurally excluded from the srv-notify call every secondary window-* close already gets (CloseWindowTask, gated `self.label != "main"` on the incorrect assumption that process exit alone cleans up srv-side state). srv never heard about the close, so its window/workspace/tab rows leaked permanently and crash-reproject resurrected them as "ghost" windows on every subsequent launch. Fixed by notifying srv synchronously (not on a background thread, which lost the race against the host's own sidecar-kill on shutdown) before main's close completes. Live-verified: db_window drops to 0 after close, including cleanup of a pre-existing leaked row. A related but non-live gap in on_before_close's Stage 2 (real on macOS/Linux, dead code for this case on Windows) was fixed alongside as defense-in-depth.
