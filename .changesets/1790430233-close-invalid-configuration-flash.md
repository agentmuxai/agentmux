---
type: patch
---
Closing the main window no longer flashes "invalid configuration, client or window was not loaded" on the way out. That message is for a window that never loaded; once a window has loaded, losing its record (as closing does) now shows the plain background instead.
