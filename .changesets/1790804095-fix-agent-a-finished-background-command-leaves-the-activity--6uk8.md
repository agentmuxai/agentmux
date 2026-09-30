---
type: patch
---

fix(agent): a finished background command leaves the Activity Dock in every pane — including one opened after an upgrade or restart, where two finished commands used to stay 'running' for hours because the dock only heard about endings from a registry that starts empty
