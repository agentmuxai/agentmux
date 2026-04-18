# SPEC: Decouple Agent Type and Provider from Seed Manifest

**Date:** 2026-04-16
**Status:** Draft

---

## Problem

The forge seed manifest (`forge-seed.json`) hardcodes two properties that
should be user-configurable:

```json
{
    "id": "agentx", "provider": "claude", "agent_type": "host", "environment": "windows",
    "id": "agenty", "provider": "codex", "agent_type": "host", "environment": "windows",
    "id": "agentz", "provider": "gemini", "agent_type": "host", "environment": "windows",
    "id": "agent1", "provider": "claude", "agent_type": "container", "environment": "linux",
    "id": "agent2", "provider": "codex", "agent_type": "container", "environment": "linux",
    "id": "agent3", "provider": "gemini", "agent_type": "container", "environment": "linux"
}
```

**`provider`** (claude/codex/gemini) — The Forge UI already has a provider
dropdown. Hardcoding it means the user must edit the agent after seeding to
change providers. It also creates a false association between agent identity
and provider choice (AgentY is not inherently "the Codex agent").

**`agent_type`** (host/container) — This is a deployment topology detail, not
an identity property. Whether an agent runs locally or in a container depends
on the runtime environment, not on which agent it is. The same agent should be
deployable either way.

**`environment`** (windows/linux) — Derived from the platform at seed time,
not a fixed property of the agent identity.

Similarly, the AWS secrets store (`services/infra → agent-configs`) also
hardcodes `template_type: "host"|"container"` per agent — redundant with
the Forge `agent_type` field.

---

## Goal

1. Seed manifest defines **identity only**: id, slug, name, icon, description
2. Provider, agent_type, and environment are set **at runtime** via the Forge
   UI or auto-detected from the platform
3. AWS agent-configs store only **credentials and infrastructure refs** — no
   deployment topology

---

## Design

### 1. Seed Manifest Changes

Remove `provider`, `agent_type`, and `environment` from `forge-seed.json`.
Add a `provider` default that maps to the first available provider on the
platform (auto-detected at seed time).

**Before:**
```json
{
    "id": "agentx",
    "name": "AgentX",
    "icon": "✦",
    "provider": "claude",
    "agent_type": "host",
    "environment": "windows",
    "description": "Primary development agent"
}
```

**After:**
```json
{
    "id": "agentx",
    "name": "AgentX",
    "icon": "✦",
    "description": "Primary development agent"
}
```

### 2. Seed Engine Changes (`forge_seed.rs`)

When a seeded agent lacks `provider`, `agent_type`, or `environment`, the
seed engine fills in sensible defaults:

| Field | Default |
|-------|---------|
| `provider` | `"claude"` (first registered provider) |
| `agent_type` | `"host"` (local machine) |
| `environment` | Auto-detected: `std::env::consts::OS` → `"windows"`, `"macos"`, `"linux"` |

This preserves backward compatibility: old seed manifests with explicit values
still work. New manifests can omit these fields.

### 3. AWS Agent-Configs Cleanup

Remove `template_type` from `services/infra → agent-configs`. The remaining
fields are pure infrastructure refs:

**Before:**
```json
{
    "agent_id": "agentx",
    "template_type": "host",
    "aws_profile": "AgentX",
    "github_app_id": "2137233",
    "github_app_installation_id": "90590829"
}
```

**After:**
```json
{
    "agent_id": "agentx",
    "aws_profile": "AgentX",
    "github_app_id": "2137233",
    "github_app_installation_id": "90590829"
}
```

### 4. Forge UI Behavior

The Forge agent card settings panel already has:
- **Provider dropdown** — selects claude/codex/gemini/openclaw/pi
- **Agent type field** — host/container

No UI changes needed. Users can change provider and agent_type at any time
after seeding, which is already supported.

### 5. Migration

Existing databases already have agents with `provider` and `agent_type` set
(from previous seeds or user edits). These values are preserved — the seed
engine only fills defaults for NEW agents.

The seed engine's update path (`update_seeded_agent_if_changed`) must NOT
overwrite user-modified `provider` or `agent_type` values. Since these fields
are being removed from the manifest, the update check will no longer see them
as "changed" — so no forced overwrites occur. This is the correct behavior.

---

## Implementation Plan

### PR 1: Seed manifest + engine

1. Remove `provider`, `agent_type`, `environment` from `forge-seed.json`
   (keep `description`, `content`, `skills`)
2. Update `forge_seed.rs`:
   - Make `provider`, `agent_type`, `environment` optional in `SeedAgent`
   - Fill defaults when absent
   - Don't include these fields in the "changed" comparison for updates
3. Update `forge_seed.rs` reseed logic to not overwrite user-set provider

### PR 2: AWS secrets cleanup

1. Remove `template_type` from all agent entries in `services/infra → agent-configs`
2. Update any code that reads `template_type` from the secret (if any)

---

## Non-Goals

- **Removing provider/agent_type from the ForgeAgent schema.** These fields
  stay — they're user-configurable properties. We're just removing the
  hardcoded defaults from the seed manifest.
- **Auto-detecting provider.** The default is always `"claude"`. Users pick
  their preferred provider in the Forge UI.
- **Changing the container agent deployment model.** Container agents still
  exist as a concept; we're just not hardcoding which agents are containers.
