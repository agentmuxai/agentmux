# Spec: "New Agent" heading + harness-only icons in the template section

**Date:** 2026-08-22
**Author:** Camper
**Status:** Draft — not implemented
**Motivated by:** direct request — two small UI cleanups to the Agent
pane's template-card section. "My Agents" is explicitly out of scope
("my agents are good").

## Problem

In the Agent pane (`AgentPicker.tsx`), below the user's own agents
("My Agents"), there's a template section for creating a new agent from
a harness (Claude Code, Codex, Gemini CLI, etc.):

1. The section header reads "+ New from template" — the request is to
   simplify this to "New Agent."
2. Each template card's icon is rendered via `DualProviderLogo`, which
   overlays a small vendor-logo badge in the corner of the harness icon
   (e.g. a tiny Anthropic mark badged onto the Claude Code icon). Since
   this section is explicitly about picking a **harness** (the hint text
   right below the header already says exactly this — "Each card is a
   harness... you'll pick which model it uses next"), the badge is
   redundant here and the request is to show just the harness icon.

## Design

Both changes are scoped to the template section only — "My Agents"
(`MyAgentsList.tsx`) is a fully separate component/data path (different
RPC, different rows) and is untouched by either change.

### 1. Header text

`frontend/app/view/agent/components/AgentPicker.tsx:898-905` — change
the `<span>` text from `"+ New from template"` to `"New Agent"`. The
hint paragraph directly below it (`.agent-picker-templates-hint`,
"Each card is a harness...") stays as-is — it's still accurate and
still useful context for a first-time user.

### 2. Harness-only card icon

`frontend/app/view/agent/components/AgentCard.tsx` (~line 122-127)
currently renders:
```tsx
<DualProviderLogo
    harness={props.agent.provider}
    vendor={resolveEffectiveVendor(props.agent.provider, props.agent.model_vendor_base_url)}
    size={28}
    class="agent-card-icon"
/>
```
Replace with a plain `ProviderLogo` — no wrapper, no badge, no vendor
resolution needed at all:
```tsx
<ProviderLogo provider={props.agent.provider} size={28} class="agent-card-icon" />
```
This is a straight swap, not a new prop on `DualProviderLogo` — nothing
else about `AgentCard`'s icon rendering needs the badge/vendor concept,
so adding a `showVendorBadge` flag to the shared component would be an
unused-elsewhere abstraction. `DualProviderLogo` itself is untouched;
`MyAgentsList.tsx` keeps using it exactly as today, badge and all,
since the user confirmed that section is fine.

`resolveEffectiveVendor`'s import in `AgentCard.tsx` becomes unused and
should be removed along with the `DualProviderLogo` import, once the
swap lands (avoid leaving a dead import).

## Non-goals

- **`MyAgentsList.tsx` / "My Agents" is untouched** — it keeps
  `DualProviderLogo` and its vendor badge exactly as today.
- **`DualProviderLogo` component itself is untouched** — this is a
  caller-side change in `AgentCard.tsx`, not a change to the shared
  component's own behavior or API.
- **No change to the hint paragraph, install ribbon, card title/caption,
  or template filtering (`is_seeded === 1`) logic** — only the header
  text and the icon.

## Testing

- `frontend/app/view/agent/components/AgentPicker.test.tsx:217-221`
  currently asserts `toHaveTextContent("New from template")` — update
  to `"New Agent"`.
- Add/update an `AgentCard` render test (or extend an existing one) to
  assert the template card's icon markup contains no
  `.dual-provider-logo-badge` element, distinguishing it from
  `MyAgentsList`'s rows, which should still render one when
  `vendor !== harness`.
- Manual: launch `task dev`, open the Agent pane, confirm the header
  reads "New Agent" and every template card (Claude Code, Codex, Gemini
  CLI, Antigravity, Kimi Code, GitHub Copilot CLI, Pi, etc.) shows only
  its own harness icon with no corner badge, while "My Agents" rows
  below an agent with a custom `model_vendor_base_url` or a
  vendor-divergent provider still show their badge unchanged.
