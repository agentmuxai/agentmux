---
type: patch
---

fix(linux): a dev build or app started by an agent no longer waits behind the AppArmor sandbox dialog: it stops within seconds with exit code 78 and a message naming AGENTMUX_UNSAFE_NOSANDBOX=1
