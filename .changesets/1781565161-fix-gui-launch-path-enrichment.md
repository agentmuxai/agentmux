---
type: minor
---

fix(toolchain): GUI-launched AgentMux can find nvm/Homebrew node, npm & git — enrich the srv's PATH from the user's login shell (+ well-known toolchain dirs) so `npm install`/agent CLIs resolve when launched from Finder/Dock/DMG (was failing with "npm: command not found"). Additive, login-shell-sourced, no-op on Windows. P0 of SPEC_TOOLCHAIN_MANAGER.
