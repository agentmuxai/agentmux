# Identity Pane — Implementation Plan (Phase 1)

**Branch:** `agenty/feat-identity-pane`
**Spec:** `docs/specs/identity-management.md`
**Scope:** Account registry UI — create/edit/delete external accounts, localStorage persistence, grouped display. No backend changes, no validation (Phase 2).

---

## Files Changed

| File | Action | Purpose |
|------|--------|---------|
| `agentmuxsrv-rs/src/config/widgets.json` | Edit | Register `defwidget@identity` |
| `frontend/app/block/block.tsx` | Edit | `BlockRegistry.set("identity", IdentityViewModel)` |
| `frontend/app/view/identity/identity-model.ts` | Create | `IdentityViewModel` — Solid.js signals, localStorage CRUD |
| `frontend/app/view/identity/identity-view.tsx` | Create | Solid.js view — accounts tab, detail panel, add/edit form |
| `frontend/app/view/identity/identity.tsx` | Create | Barrel — wires `viewComponent` onto prototype |
| `frontend/app/view/identity/identity-view.scss` | Create | Styles matching swarm/forge conventions |

## Architecture Decisions

**Storage:** `localStorage['agentmux:identity:accounts']` — JSON array of Account objects.
No backend changes in Phase 1. WaveStore/SQLite migration deferred to Phase 2 when validation is added.

**Framework:** Solid.js signals (`createSignal`, `For`, `Show`) — identical to swarm/forge pattern.

**Status:** All accounts show `"unknown"` in Phase 1 — validation engine is Phase 2.

## Data Model

```typescript
interface Account {
  id: string;                          // UUID
  name: string;                        // Display name "GitHub agent1-workflow"
  provider: 'github' | 'aws' | 'anthropic' | 'custom';
  kind: 'pat' | 'role' | 'api_key' | 'env_ref';
  display_name?: string;               // GitHub username, ARN alias, etc.
  secret_ref: SecretRef;
  context: AccountContext;
  assigned_agents: string[];
  status: AccountStatus;               // always 'unknown' in Phase 1
  created_at: string;
  updated_at: string;
}
```

## UI Structure

```
identity-view
├── header (title + "+ Add Account" button)
├── tabs [Accounts | Assignments]
└── content
    ├── Accounts tab
    │   ├── provider group: GITHUB
    │   │   └── account row × n
    │   ├── provider group: AWS
    │   ├── provider group: ANTHROPIC
    │   └── provider group: CUSTOM
    ├── Assignments tab (agent → accounts matrix, Phase 1 read-only)
    └── detail panel (shown when account selected)
        ├── account name + status badge
        ├── provider icon + display_name
        ├── secret_ref display (masked)
        ├── context fields
        ├── assigned_agents list
        └── [Edit] [Delete] buttons
```

## Steps

1. Add `defwidget@identity` to `widgets.json`
2. Write `identity-model.ts` (ViewModel + localStorage CRUD)
3. Write `identity-view.tsx` (accounts list + detail panel + add/edit form)
4. Write `identity.tsx` (barrel)
5. Write `identity-view.scss`
6. Register in `block.tsx`
7. Bump version
8. Commit + push + PR

## Out of Scope (Phase 2)

- Token validation (GitHub /user, AWS STS, Anthropic /v1/models)
- Backend WaveStore migration
- Forge identity section
- Swarm identity badges
- Agent assignment at launch (env injection)
