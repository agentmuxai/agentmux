# Default Agent Roster Spec

**Date:** 2026-03-17
**Status:** Ready to implement

---

## Agent Roster

Six default agents seeded in the Forge: 3 host-native, 3 container-based.

### Host Agents

| Slot | Name | Provider | Type | Description |
|------|------|----------|------|-------------|
| X | AgentX | claude | host | Claude Code on host — primary coding agent |
| Y | AgentY | codex | host | Codex CLI on host — OpenAI coding agent |
| Z | AgentZ | gemini | host | Gemini CLI on host — Google coding agent |

### Container Agents

| Slot | Name | Provider | Type | Description |
|------|------|----------|------|-------------|
| 1 | Agent1 | claude | container | Claude Code in container — sandboxed coding |
| 2 | Agent2 | codex | container | Codex CLI in container — sandboxed coding |
| 3 | Agent3 | gemini | container | Gemini CLI in container — sandboxed coding |

### Removed

- **Agent4** — drop (was: unused placeholder)
- **Agent5** — drop (was: unused placeholder)

---

## ForgeAgent Field Mapping

```typescript
// Host agents
{ name: "AgentX", icon: "✖", provider: "claude",  agent_type: "host",      environment: "local" }
{ name: "AgentY", icon: "✦", provider: "codex",   agent_type: "host",      environment: "local" }
{ name: "AgentZ", icon: "⚡", provider: "gemini",  agent_type: "host",      environment: "local" }

// Container agents
{ name: "Agent1", icon: "①", provider: "claude",  agent_type: "container", environment: "docker" }
{ name: "Agent2", icon: "②", provider: "codex",   agent_type: "container", environment: "docker" }
{ name: "Agent3", icon: "③", provider: "gemini",  agent_type: "container", environment: "docker" }
```

---

## Implementation Notes

### Seeding

The Forge seeds default agents on first run (or when `is_seeded` is set). Update the seed list to:
1. Remove Agent4 and Agent5 entries
2. Add/update the 6 agents above
3. Set `is_seeded: 1` on all

### Container Support

Container agents (`agent_type: "container"`) require:
- Docker/Podman runtime available on host
- CLI installed inside the container image
- Working directory mounted from host
- Separate auth context per container (handled by per-agent config isolation from PR #154)

### 3-Pane Layout

The tighter UI density (PR #151) was specifically designed to support viewing 3 agents simultaneously. Recommended default layout: 3 vertical columns showing AgentX, AgentY, AgentZ (host agents) for the primary workflow.

### Provider Coverage

Each provider appears exactly twice — once on host, once in container:

| Provider | Host | Container |
|----------|------|-----------|
| Claude Code (`claude`) | AgentX | Agent1 |
| Codex CLI (`codex`) | AgentY | Agent2 |
| Gemini CLI (`gemini`) | AgentZ | Agent3 |

This gives full coverage for comparing provider behavior in both environments.
