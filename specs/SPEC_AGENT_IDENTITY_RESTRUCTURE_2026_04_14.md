# Spec: Agent identity restructure — two names, easy rename, external usernames

**Status:** Draft
**Date:** 2026-04-14
**Scope:** `ForgeAgent` data model, agent picker UX, `IdentityViewModel`
account storage, launch-side working-directory / env-var derivation
**Related:** `SPEC_CONSOLIDATE_FORGE_IDENTITY_INTO_AGENT_2026_04_13.md`,
`SPEC_AGENT_PANE_FOLLOWUPS_2026_04_13.md`

---

## 1. Problem

Agents today have **one name field** that is asked to do three incompatible jobs:

1. **Display name** shown in the agent picker, the pane frame title, the
   notification tray. Wants to be human, pretty, renameable.
2. **Stable identity** used to derive working directories, env vars,
   GitHub CLI config dirs, and cross-references from memories and
   plans. Wants to never change.
3. **External identity** (GitHub username, AWS profile, Anthropic account)
   used when the agent actually talks to a service. Wants to be
   multi-valued and provider-specific.

The single `ForgeAgent.name` field is tied to all three at launch time
(see `agent-model.ts:236-270`):

```ts
// Working directory slug — derived from name
const workDir = agent.working_directory
    || `~/.agentmux/agents/${agent.name.toLowerCase().replace(/[^a-z0-9-_]/g, "-")}`;

// GitHub CLI config dir — derived from name
const agentSlug = agent.name.toLowerCase().replace(/[^a-z0-9-_]/g, "-");
envVars["GH_CONFIG_DIR"] = `~/.agentmux/config/gh-${agentSlug}`;

// Env var that downstream MCPs + claw read as "who am I"
envVars["AGENTMUX_AGENT_ID"] = agent.name;
```

Consequences:

- **Renaming an agent is dangerous.** Changing "AgentX" → "Alex" would
  silently orphan `~/.agentmux/agents/agentx/`, break `GH_CONFIG_DIR
  =~/.agentmux/config/gh-agentx/`, and break every memory / plan file
  that refers to "AgentX" by name. There's no rename UX at all right
  now — you have to delete and re-create the agent, which is exactly
  why the existing forge seed hardcodes five Agent1-5's: nobody dares
  rename them.
- **There is no place to record external usernames.** The `IdentityViewModel`
  has `AccountContext` with `github_username`, `aws_profile`, etc., but
  the storage is a flat `localStorage` list of accounts (`Account[]`),
  not linked to any particular `ForgeAgent`. Clicking 👤 on an agent
  card opens a panel that shows *all* accounts in the system, with an
  `assigned_agents: string[]` back-reference. It works, but the
  coupling is loose and the agent has no `githubUser` / `awsProfile`
  direct field. So a user who wants to rename "AgentX" to "Alex" but
  keep the underlying GitHub user `a5af-agentx` and AWS profile
  `agentx-dev` is stuck.
- **`AGENTMUX_AGENT_ID` serves two roles.** Downstream code treats it
  as both the agent's human-readable display AND its stable identifier
  — so CLAUDE.md templates say "You are **AgentX**" (display) while
  inter-agent MCP messaging routes via `agent.name` as a primary key
  (stable). Rename breaks one or the other.

## 2. What the user asked for

Restating to make sure the restructure matches intent:

1. **An agent has two names:**
   - An **AgentMux identity** (the one inside the app) — must be
     unique within an AgentMux instance, and renameable.
   - **External username(s)**, set in the Identity view — currently only
     GitHub and AWS fields exist there.
2. **Rename should be easy.** One click, inline on the card or in the
   agent settings panel. No breakage.
3. **Keep external identity separate.** The GitHub username, AWS
   profile, etc. live in the Identity view and point *to* an agent,
   not the other way around.

Plus one side question about two buttons on the agent pane that "do
nothing except show a blue line" — addressed as a separate issue in
§8 below, not a scope item here.

## 3. Target model

### 3.1 ForgeAgent gains a stable slug

Add a new field `slug: string` to `ForgeAgent`. This is the forever-ID
used by:

- Working directory path (`~/.agentmux/agents/<slug>/`)
- GitHub CLI config dir (`~/.agentmux/config/gh-<slug>/`)
- Per-agent auth dir
- Backend references (memories, plans, message routing)

`slug` is set **once at creation** (typically to
`name.toLowerCase().replace(/[^a-z0-9-_]/g, "-")`), never changes, and
is surfaced in the UI as a small secondary line under the display
name — like how Slack shows `@handle` below a display name.

`name` keeps its current semantics but becomes pure display: shown
to humans, used in the agent picker, in the pane frame title, and
substituted into `{{AGENT_DISPLAY}}` in CLAUDE.md templates. **Renamable
at any time** without side effects.

New invariants (enforced in the backend):

| Field | Required | Unique? | Renameable? | Source of truth |
|---|---|---|---|---|
| `slug` | yes | yes (within AgentMux instance) | **NO** | set on create |
| `name` | yes | yes (within AgentMux instance) | **YES** | editable in UI |

**Why both must be unique:** slug because it's the key; name because
picking two agents named "AgentX" in the list is worse than forbidding
it. The uniqueness check on rename is cheap (single in-memory scan
over `ForgeAgent[]`).

### 3.2 `AGENTMUX_AGENT_ID` becomes the slug, not the name

Current launch code (`agent-model.ts:271`):

```ts
envVars["AGENTMUX_AGENT_ID"] = agent.name;
```

becomes:

```ts
envVars["AGENTMUX_AGENT_ID"] = agent.slug;
envVars["AGENTMUX_AGENT_DISPLAY"] = agent.name;
```

Downstream consumers (MCP servers, claw, memory routing) migrate to
read `AGENTMUX_AGENT_ID` as the stable slug. The display name is
available for prompt templates via `AGENTMUX_AGENT_DISPLAY`.

The claw host template (`a5af/claw:templates/host/CLAUDE.md`) already
uses `{{AGENT_DISPLAY}}` — this spec doesn't touch that, but once the
AgentMux seed → claw bridge lands (see the sibling report
`docs/analysis/agentx-git-identity-2026-04-14.md`), claw can source
`AGENT_DISPLAY` from `AGENTMUX_AGENT_DISPLAY` and stay stable through
renames.

### 3.3 External identity lives on the agent record as references

Current Identity storage (`identity-model.ts:9-45`) has:

```ts
interface Account {
    id: string;
    provider: "github" | "aws" | "anthropic" | "custom";
    context: { github_username?, aws_profile?, ... };
    assigned_agents: string[];
    ...
}
```

— i.e., accounts own a list of agent IDs they're assigned to. That's
backwards for the "rename an agent, keep its external username" flow
the user asked for, because the agent has no direct pointer to its
accounts.

**Target:** agents reference accounts by ID, not the other way
around:

```ts
interface ForgeAgent {
    id: string;           // existing — internal DB primary key (unchanged)
    slug: string;         // NEW — stable, unique, filesystem-safe
    name: string;         // existing — display, renameable
    icon: string;
    provider: string;
    description: string;
    working_directory: string;
    agent_type: string;
    // ...existing fields...

    /**
     * Per-provider external account IDs, keyed by provider. Each
     * value is an Account.id from the Identity store.
     */
    accounts: Record<AccountProvider, string | null>;
    // e.g. { github: "acct-abc123", aws: "acct-def456", anthropic: null, custom: null }
}
```

`Account.assigned_agents[]` is deprecated and derived from the
reverse index when needed (for the Identity view's "which agents
use this account?" display).

**Why this direction:** when you rename the agent, its accounts pointer
stays valid. When you delete an account, the agent's `accounts[provider]`
becomes null but the agent itself survives. When you swap accounts (e.g.
"use my prod GitHub token instead of dev for AgentX"), you just repoint
`accounts.github`. All O(1) operations, no string matching.

### 3.4 Identity panel on the agent card becomes agent-scoped

Currently, clicking 👤 on an agent card opens a panel showing *all*
accounts in the system and lets you pick which to assign to this
agent. The UX target is the opposite:

- **Default view:** the panel shows "AgentX uses **github: @a5af-agentx** and **aws: agentx-dev**" and lets you swap either.
- **Swap UI:** a dropdown per provider, listing all accounts of that
  provider, with a "+ Create new" at the bottom that opens the
  existing form inline.
- **Global list** (the current default view) moves to a separate tab
  or is deleted. We can bring it back later if needed — it's not the
  primary way people interact with accounts.

## 4. Rename UX

### 4.1 Where rename lives

Two entry points, both opening the same modal:

1. **Inline from the agent card** — a tiny ✏ button next to the name,
   visible on hover. Clicking replaces the name span with an
   `<input>` + ✓/✗ buttons. Enter saves, Esc cancels. Same pattern as
   the notebook tab rename in `tabmanager.tsx`.
2. **From the agent settings panel** (the one that opens on ⚙): a
   "Display name" field at the top. Changing it and hitting save
   renames the agent.

### 4.2 What rename does / doesn't change

| Field | Before rename | After rename |
|---|---|---|
| `slug` | `agentx` | `agentx` (unchanged) |
| `name` | `AgentX` | `Alex` |
| Working directory | `~/.agentmux/agents/agentx/` | same (because it's derived from slug) |
| `GH_CONFIG_DIR` | `~/.agentmux/config/gh-agentx/` | same |
| `AGENTMUX_AGENT_ID` env | `agentx` | same |
| `AGENTMUX_AGENT_DISPLAY` env | `AgentX` | `Alex` |
| Pane frame title | `AgentX` | `Alex` (live; see blockAtom in `agent-model.ts:44`) |
| Agent picker card | `AgentX` | `Alex` |
| Memories, assignments, plans | reference `agentx` | still valid |
| Linked GitHub / AWS accounts | `accounts.github = "acct-abc"` | still valid |

Running panes update live on the next reactive cycle — `agent-model.ts`
already reads `agentName` from block meta, so setting the new name
in the agent's record + notifying open panes is enough.

### 4.3 Validation

On save, enforce:

1. Name not empty, not whitespace-only, length ≤ 64.
2. Name is unique among existing ForgeAgent names (case-insensitive).
3. No-op if the new name equals the old name (don't bump `updated_at`).

Slug is never edited in this flow.

### 4.4 Edge case — creating a new agent

When the user creates a new agent via the `+` card, the form should:

1. Ask for a **display name** first.
2. Autogenerate a slug from that name (lowercase, dash, punctuation
   stripped).
3. Show the slug in small text below the name input, with a "✏ edit
   slug" affordance for power users who want a different filesystem
   name. Editing the slug is allowed *only at creation*; after
   creation it's immutable.
4. If the autogenerated slug collides with an existing one, append
   `-2`, `-3`, etc. until unique, and show a hint to the user.

This matches how most services handle handle generation (GitHub,
Slack, Discord all let you pick once, change later only with
intervention).

## 5. Implementation steps

Each step is a separate PR. Steps 1-3 are reversible data/UI
changes; step 4 is the external-identity coupling.

### Step 1 — Backend: add `slug` column

- **Touch:** `agentmux-srv/src/backend/storage/wstore.rs` (or whatever
  owns `ForgeAgent`), `forge-seed.json`, migration logic.
- Add `slug: String` to the `ForgeAgent` row schema.
- Backfill migration: for every existing row, derive
  `slug = name.to_lowercase().filter(is_filesystem_safe).take(64)`,
  collision-resolve with `-2`, `-3`.
- Update the seed manifest (`forge-seed.json`) to include `slug` on
  each entry.
- Uniqueness constraint on `slug` at the storage layer.
- Unit tests for the slug derivation + collision resolution.

### Step 2 — Launch: use slug for filesystem paths

- **Touch:** `frontend/app/view/agent/agent-model.ts:236-271`,
  backend `WriteAgentConfigCommand` handler.
- Change `workDir` default to
  `~/.agentmux/agents/${agent.slug}/` (no fallback to `name`).
- Change `GH_CONFIG_DIR` derivation to `gh-${agent.slug}`.
- Change `envVars["AGENTMUX_AGENT_ID"]` to `agent.slug`.
- Add `envVars["AGENTMUX_AGENT_DISPLAY"] = agent.name`.
- Template variable expansion: add `{{AGENT_SLUG}}` alongside the
  existing `{{AGENT}}` / `{{AGENT_DISPLAY}}` / `{{AGENT_ID}}`.
- **Behavior-preserving migration.** Step 1 already backfilled
  `slug = name.toLowerCase()` for existing agents, so the paths
  don't change for anyone in practice — the derivation just stops
  going through the `.toLowerCase().replace(...)` call site and
  reads the stored field directly.

### Step 3 — UI: rename widget

- **Touch:** `AgentCard.tsx`, `AgentCardSettingsPanel.tsx`, a new small
  `InlineRenameField.tsx` (or reuse an existing rename primitive if
  there's one — check `tabmanager.tsx`, `ForgeDetail.tsx`).
- Add ✏ hover button to `AgentCard`.
- Add "Display name" field to the Forge tab of the settings panel.
- Wire save → `forge:updateAgent` RPC with `{id, name}`.
- Validation client-side (empty, duplicate) before the RPC.
- Show the slug in small grey text under the name in both the card
  and the settings panel. Not editable.
- Live-update open panes: since `agent-view.tsx` already reads
  `agentName` from block meta, the rename handler also needs to push
  `agentName` into block meta for any block currently running that
  agent. Skip the update if no pane has this agent open.

### Step 4 — Identity: agent-owned account refs

- **Touch:** `identity-model.ts`, `forge-seed.json`, backend storage,
  `IdentityPanel` component.
- Add `accounts: Record<AccountProvider, string | null>` to
  `ForgeAgent`. Seed with `{github: null, aws: null, anthropic: null, custom: null}`
  for existing rows.
- Deprecate `Account.assigned_agents[]`: keep the field for now but
  derive it from the reverse index at query time. Schedule a removal
  follow-up.
- Rewrite the Identity panel opened from the agent card:
  - Top: "AgentX uses the following accounts:"
  - Per provider row: the currently-assigned account (if any) +
    a dropdown to pick a different one + "+ New" to open the
    existing account-create form inline.
  - Unassign is a button that sets `accounts[provider] = null`.
- Surface the GitHub username / AWS profile from the assigned
  account inline on the agent card (the user asked for external
  usernames to be visible under identity).

### Step 5 — Polish + docs

- Update the `CLAUDE.md` in `a5af/claw:templates/host/` to read
  from `AGENTMUX_AGENT_DISPLAY` instead of hardcoding the agent name.
  (Separate PR on the claw side — out of scope for AgentMux but
  tracked in the migration notes.)
- Add a section to `docs/analysis/agentx-git-identity-2026-04-14.md`
  that cross-references this spec — once slug lands, claw can trust
  that `~/.agentmux/agents/<slug>/` never moves.
- Write a short "How to rename an agent" note in the main README.

### Estimated cost

| Step | Time | Risk |
|---|---:|---|
| 1. Backend slug + migration | 2h | Low — purely additive |
| 2. Launch-side derivation | 1h | Low — value-preserving |
| 3. Rename UI + validation | 2h | Low |
| 4. Identity panel restructure | 3-4h | Medium — reshapes the Account ↔ agent link |
| 5. Docs + claw follow-up | 30 min | Low |

**Total: ~9 hours**, four PRs, shippable independently.

## 6. Out of scope

- Multi-agent-instance sync (Identity accounts cross AgentMux
  installations). Each instance keeps its own localStorage for now.
- Renaming the `slug` after creation. Deliberately forbidden — the
  whole point of the split is that *something* is stable.
- Importing the user's existing git global config as the default
  GitHub username on account creation. Nice, but not required.
- The claw host-template update to use `AGENTMUX_AGENT_DISPLAY` —
  that's a claw-side PR tracked separately.

## 7. Open questions

1. **Slug character set:** allow hyphens only, or also underscores?
   Current derivation uses `[^a-z0-9-_]`. Keep as-is unless there's
   a reason to tighten.
2. **Rename in the running pane:** should the pane title update live
   (subject to the blockAtom reactive cycle) or only on next launch?
   Leaning live — the reactive plumbing already exists.
3. **What happens when the seed manifest bumps a name?** e.g. if a
   future seed manifest renames `AgentX` → `AgentXReboot`. Leave it
   to the seeder's usual "skip if slug exists" guard, which is already
   the logic in `forge_seed.rs:seed_forge_agents`. The seeder never
   overwrites existing rows — users who want the new name rename
   manually.
4. **AgentMux UUID vs slug as the filesystem key:** a UUID would be
   100% collision-free but less debuggable (`~/.agentmux/agents/f8d2-...`).
   Slug wins on debuggability. Collision handling via `-2`, `-3`
   suffix is a known technique.

## 8. Side question — the two "blue line" buttons on the agent card

Investigating what the user saw: the two buttons on the agent *picker*
card are ⚙ (Forge) and 👤 (Identity), defined in `AgentCard.tsx:65-84`.
Their handlers (`onOpenForge` / `onOpenIdentity` in `AgentPicker.tsx:94-119`)
set an `expandedId` / `expandedTab` signal that causes
`AgentCardSettingsPanel` to render inline below the card. So they
*should* do something — they toggle an inline settings panel with
Forge / Identity tabs.

**If the user is seeing only "a blue line"** after clicking, it means
the panel is rendering but its *body* is empty or collapsed to
zero height. Likely causes, ranked:

1. `AgentCardSettingsPanel.tsx:120` wraps the body in `<Show when={tab() === "forge"}>` / `<Show when={tab() === "identity"}>`. If neither tab matches (e.g., `initialTab` is garbage), both `<Show>` branches return null and the panel renders just its header — which, styled with a border-bottom, looks exactly like "a blue line."
2. `ForgeDetail` or `IdentityPanel` throws during mount, gets caught by
   an error boundary higher up, and silently renders nothing. Check
   the `[fe]` log in the dev server terminal for any `console.error`
   after clicking the button.
3. SCSS: `.agent-card-settings-body` has `max-height: 0` or
   `overflow: hidden` and its content never expands. Scan
   `agent-view.scss` for `agent-card-settings-body`.

**This is a real bug** — the buttons do have an intent (expand the
inline settings). They're not meant to open a new pane. Recommend a
separate fix PR after the restructure spec lands, because the Identity
panel shape is going to change in Step 4 anyway and fixing the render
bug in today's shape would be thrown-away work.

**Quick triage step to confirm:**

```bash
# In the dev server terminal tailing `[fe]` log:
muxlog host '[fe]' &
# Click the ⚙ button on AgentX. Look for:
#   - any console.error / stack trace
#   - any "Cannot read properties of ..." message
```

If the log is clean, the problem is SCSS / `<Show>` gating, which is
a 5-minute CSS fix. If there's an error, it's model instantiation
inside `AgentCardSettingsPanel` — hand it to the same PR that
rewrites the panel in Step 4.

## 9. Success criteria

After all five steps land:

- Creating a new agent through the `+` card lets me type a display
  name; the slug is auto-generated and shown in small text; I can
  override it once before save.
- Clicking ✏ on an existing card lets me rename the display name
  without touching any file on disk or breaking any launched pane.
- Trying to rename AgentX to "Agent1" (duplicate) shows an error
  inline in the input and doesn't save.
- After a rename, the pane frame title and the picker card show the
  new name; the working directory, env vars, and MCP routing still
  point at the old slug.
- Clicking 👤 on an agent card shows *that agent's* GitHub and AWS
  accounts (if any), with dropdowns to swap and a "+ New" to add.
- Deleting an assigned account unlinks it from the agent but leaves
  the agent operational (the rows just show "no account assigned").
- The `CLAUDE.md` generated at launch still says "You are **AgentX**"
  (from the display name) but downstream inter-agent messaging keys
  by `agentx` (from the slug) and is stable across renames.

---

## Appendix: exact file list

**Data model:**
- `agentmux-srv/src/backend/storage/wstore.rs` — ForgeAgent schema
- `agentmux-srv/src/backend/storage/migrations.rs` — slug backfill
- `agentmux-srv/forge-seed.json` — add slug to every entry

**Launch path:**
- `frontend/app/view/agent/agent-model.ts:236-271` — derivations
- `frontend/app/view/agent/agent-model.ts:380-487` — template vars

**UI components:**
- `frontend/app/view/agent/components/AgentCard.tsx` — rename affordance
- `frontend/app/view/agent/components/AgentCardSettingsPanel.tsx` — rename field
- `frontend/app/view/agent/components/IdentityPanel.tsx` — agent-scoped view
- New: `frontend/app/view/agent/components/InlineRenameField.tsx`

**Identity store:**
- `frontend/app/view/identity/identity-model.ts` — deprecate
  `assigned_agents`, add reverse index
- `frontend/app/view/identity/identity-view.tsx` — "which agents use
  this account?" section (derived)

**Docs:**
- `README.md` — how to rename
- `docs/analysis/agentx-git-identity-2026-04-14.md` — cross-reference
