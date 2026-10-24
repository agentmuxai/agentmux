---
type: minor
---

Claude agents now compact their context at 600K tokens instead of near a full 1M window, set by the new Settings > Advanced > Compact agents at (agent:compactat; 0 keeps Claude Code's own). AgentMux writes it into each Claude config it prepares and leaves a value a person set alone.
