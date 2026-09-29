---
type: patch
---

fix(linux): AgentMux no longer launches with an invisible window on a VM or GPU without working 3D; it checks the GPU really renders before forcing hardware GL, and falls back to software otherwise
