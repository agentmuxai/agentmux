---
type: minor
---

feat(identity): non-stable channels (task dev, task package) now default to isolated per-channel auth

Every `task dev` branch and local `task package` build now starts with a genuinely empty identity/OAuth credential store instead of sharing the real global account list — you'll need to reconnect providers per branch/build. The `stable` release channel is unaffected. Set `AGENTMUX_ISOLATED_AUTH=0` before `task dev`/`task package` to opt back into the old global-sharing behavior for that session. See `docs/specs/SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md`.
