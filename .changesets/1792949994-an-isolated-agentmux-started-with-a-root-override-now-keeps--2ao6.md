---
type: patch
---

An isolated AgentMux (started with a root override) now keeps each version's data in its own versions folder and carries it forward on upgrade, so upgrades can be tested without touching a real install
