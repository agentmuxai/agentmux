---
type: patch
---

feat(agent): recency-sorted launch picker + updated_at on agent definitions

**Recency-sorted picker.** `agent_def_list` previously ordered by
`created_at ASC` (oldest first). It now orders **most-recently-used first**:
a `LEFT JOIN` on `MAX(db_agent_instances.started_at)` per definition. Never-
launched agents (no instance rows → NULL `last_used`) sort after the launched
ones under `DESC`, ordered among themselves by `created_at ASC`. The
AgentPicker reflects this order directly.

**Default selection.** The AgentPicker now focuses the first card (the
most-recently-used agent) on mount via a new `AgentCard.defaultFocus` prop —
so the focus ring marks the default and Enter launches it immediately.

**`updated_at` on agent definitions.** `db_agent_definitions` gains an
`updated_at` column (schema v2 — `OBJECT_SCHEMA_VERSION` bumped, with an
idempotent `ALTER TABLE ADD COLUMN` for existing dev databases). It is set to
`created_at` on insert and stamped fresh on every `agent_def_update`. Surfaced
on the `AgentDefinition` struct + `gotypes.d.ts` type. (`db_memory_bundles` /
`db_identity_bundles` already had `updated_at`; agent definitions did not.)
