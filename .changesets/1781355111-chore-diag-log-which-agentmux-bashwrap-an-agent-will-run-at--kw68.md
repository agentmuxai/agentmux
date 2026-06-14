---
type: patch
---

chore(diag): log which agentmux-bashwrap an agent will run at spawn — info when the bundled (version-locked) binary is used, WARN when it's missing and the agent will fall through to a possibly-stale system-PATH copy. Cross-check with agentmux-bashwrap --version (already supported). Cheap guardrail for the stale-binary trap (RETRO_BASHWRAP_STALE_BUNDLE_2026_06_13)
