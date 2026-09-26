---
type: patch
---

Take over between two AgentMux instances on one computer is reliable when the other instance holds the agent only through AgentMux cloud: the other instance lets go of the cloud lease before answering, the pane's retry is no longer refused by a stale 'running elsewhere' answer, and a lease renewal already under way can't claim the agent straight back.
