---
type: patch
---

fix(build): agents always run the app's own bashwrap, not a stale system-PATH copy — dev build now bundles agentmux-bashwrap into the runtime tools/bin, and the sidecar PREPENDS the bundled (version-locked) tools dir to the agent PATH instead of appending it (was the exit-130 root cause: a stale Downloads-portable bashwrap on the system PATH shadowed the fixed bundled one)
