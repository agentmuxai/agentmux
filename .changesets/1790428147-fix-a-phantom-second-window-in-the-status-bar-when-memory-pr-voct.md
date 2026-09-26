---
type: patch
---

Fix a phantom second window in the status bar: when memory pressure trimmed a warm pool window that had been used as a real window and closed, it stayed registered with no window behind it. It is now unregistered directly.
