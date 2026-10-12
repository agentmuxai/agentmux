---
type: patch
---

fix(srv): shut down when the launcher exits even though its stderr is gone; macOS 'Quit & Reopen' no longer leaves srv running and the app unopenable
