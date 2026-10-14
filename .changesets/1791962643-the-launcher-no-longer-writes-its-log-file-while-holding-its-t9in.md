---
type: patch
---

The launcher no longer writes its log file while holding its state lock on every window move, and a login start that keeps the first window hidden now marks AgentMux as running unattended in the same place as closing every window does.
