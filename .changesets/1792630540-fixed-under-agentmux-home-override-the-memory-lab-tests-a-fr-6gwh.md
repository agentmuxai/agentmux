---
type: patch
---

Fixed: under AGENTMUX_HOME_OVERRIDE (the memory lab, tests), a fresh install never started because the launcher's data-update step migrated a different folder than srv opens; srv's own log under an override also went to the real home.
