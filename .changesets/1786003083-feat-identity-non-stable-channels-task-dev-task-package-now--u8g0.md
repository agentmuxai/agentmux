---
type: minor
---

feat(identity): non-stable channels (task dev, task package) now default to isolated Armory accounts

Every `task dev` branch and local `task package` build now starts with a genuinely empty Armory account list instead of sharing the real global account list — you'll need to reconnect Armory-bound providers per branch/build. This does not affect default (non-identity-bound) agent spawns, which keep resolving auth from the same global provider credential dir as before. The `stable` release channel is unaffected either way. Set `AGENTMUX_ISOLATED_AUTH=0` before `task dev`/`task package` to opt back into the old global-sharing behavior for that session. See `docs/specs/SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md`.
