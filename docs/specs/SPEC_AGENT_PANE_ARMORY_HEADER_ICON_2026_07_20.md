# SPEC: Vault Icon on the Agent-Setup Button + Responsive Tabs in the Per-Agent "Armory"

**Date:** 2026-07-20 (corrected 2026-07-21)
**Status:** Implemented
**Scope:** `frontend/app/view/agent/agent-model.ts`,
`frontend/app/view/agent/components/AgentSetupModal.tsx` / `.scss`

---

## 0. Correction note

The first version of this spec (and its initial implementation) misread the
ask as "add a **new** header icon that opens the **global** Armory pane, and
make the pane **header** itself narrow-width responsive." That was wrong on
both counts:

- There is no new icon and no new "open the global Armory" action. The
  existing "Agent setup" (`id-card`) icon **gets restyled to the vault icon**
  — same button, same click handler, icon only.
- "The armory" in the original ask refers to **`AgentSetupModal`** — the
  tabbed Accounts/Memories/MCP Servers/Skills modal that icon already opens,
  informally called "the agent armory" (a per-agent-scoped analogue of the
  global Armory pane) — not the global Armory pane itself. All the
  thinner-panes discussion was about making *that modal* degrade the way the
  global Armory pane does, not about the pane header.

The pane-header changes from the first pass (a second `endIconButtons` entry,
a `block.scss` container-query + hide-priority system) have been reverted in
full. Nothing in `blockframe.tsx`/`block.scss` is touched by this spec anymore
— see §5 for confirmation of that reduced blast radius.

---

## 1. Ask (corrected)

The existing "Agent setup" icon (top-right of an agent pane, opens
`AgentSetupModal`) should simply **use the vault icon** instead of `id-card` —
same icon Armory uses elsewhere, since this modal is effectively a per-agent
Armory. Separately, `AgentSetupModal` itself should support thinner widths the
way the global Armory pane does (it can genuinely get narrow, since it caps at
`92vw` of the app window, not a fixed size).

---

## 2. Current state (investigated)

### 2.1 The existing button — `agent-model.ts:141-152`

```ts
this.endIconButtons = () => {
    const agentId = this.blockAtom()?.meta?.["agentId"];
    if (!agentId) return [];
    return [
        { elemtype: "iconbutton", icon: "id-card", title: "Agent setup",
          click: () => { this._openAgentSetupModal?.(); } },
    ];
};
```

Hidden until an agent is loaded (empty array on the picker screen). Icons are
rendered through the shared `IconButton` component
(`frontend/app/element/iconbutton.tsx:12`) via the declarative `IconButtonDecl`
type — `icon: "vault"` resolves to `fa fa-solid fa-vault fa-fw` via
`makeIconClass`. Changing the icon is a one-line edit; the click handler,
gating, and everything else about the button stays exactly as-is.

### 2.2 What it opens — `AgentSetupModal.tsx`, "the agent armory"

```
frontend/app/view/agent/components/AgentSetupModal.tsx
```

A modal (opened via the global `useModalLayer()`, not confined to the
originating pane's DOM bounds — `agent-view.tsx:194-218`) with a horizontal
top tab bar and four tabs, each delegating to an existing standalone panel:

| Tab id | Label | Delegates to |
|---|---|---|
| `accounts` | Accounts | `AgentIdentityModalPanel` |
| `memory` | Memories | `AgentNativeMemoryModal` |
| `mcp` | MCP Servers | `AgentMcpModal` |
| `skills` | Skills | `AgentSkillsModal` |

Before this change, the tab bar was **text-only** (`{tab.label}`, no icons at
all), and the modal (`.agent-setup-modal`, `AgentSetupModal.scss:9-20`) had a
fixed `width: 780px; max-width: 92vw;` with **no responsive/container-query
handling whatsoever** — the tab bar just sat there at whatever width the
92vw cap left it, with no adaptation.

### 2.3 The reference pattern — the global Armory pane's rail

`frontend/app/view/armory/armory-view.tsx` / `.scss` — same four concepts
(plus a fifth, Bundles, which has no per-agent equivalent) as a vertical rail
with icon + label:

```ts
const RAIL: { id: ArmorySection; label: string; icon: string }[] = [
    { id: "accounts", label: "Accounts",    icon: "key" },
    { id: "brain",    label: "Memories",    icon: "brain" },
    { id: "skills",   label: "Skills",      icon: "wand-magic-sparkles" },
    { id: "mcp",      label: "MCP Servers", icon: "plug" },
    { id: "memories", label: "Bundles",     icon: "layer-group" },
];
```

Markup per item: `<i class="fa-sharp fa-solid fa-{icon}" aria-hidden="true"
/><span>{label}</span>` — the label lives in its own `<span>` specifically so
CSS can hide just the text and keep the icon.

Responsiveness is pure CSS `@container`, no JS/`ResizeObserver`:
- `.armory-container` (a wrapper `armory-view.tsx:30-31` needs, since a
  container can't query its own width) carries
  `container-type: inline-size; container-name: armory;`
  (`armory-view.scss:9-14`).
- `@container armory (max-width: 767px)` — compress the rail from `168px` to
  `48px`, hide `span` labels, icon-only.
- `@container armory (max-width: 479px)` — swap layouts entirely: hide the
  rail, show a bottom tab bar instead (always in the DOM, toggled by
  `display`).

`AgentSetupModal`'s tab bar is **already** a horizontal row (Armory's *narrow*
fallback shape), so only the label-hiding half of Armory's pattern applies —
there's no rail-to-swap transition needed since it never had a rail to begin
with.

---

## 3. Implementation (done)

### 3.1 Icon swap — `agent-model.ts`

`icon: "id-card"` → `icon: "vault"`. No other change to that button.

### 3.2 Icons + responsive tabs — `AgentSetupModal.tsx` / `.scss`

- `SetupTabDef` gained an `icon: string` field; each tab now carries the same
  icon as its matching Armory-rail concept (`key` / `brain` / `plug` /
  `wand-magic-sparkles`) — visual parity with the pane this modal is the
  per-agent analogue of.
- Tab button markup now matches Armory's exactly: `<i class="fa-sharp
  fa-solid fa-{icon}" aria-hidden="true" /><span>{label}</span>`, plus a
  native `title={label}` on the button (Armory uses a `<Tooltip>` wrapper for
  this; a native `title` attribute was used here instead to avoid pulling in
  that component for one tab bar — same information, simpler dependency
  footprint).
- `.agent-setup-modal` gained
  `container-type: inline-size; container-name: agent-setup;` — it's already
  the right ancestor for `.agent-setup-modal-tab` (a descendant), so unlike
  Armory this needed no extra wrapper `<div>`.
- One breakpoint, `@container agent-setup (max-width: 560px) { .agent-setup-modal-tab
  span { display: none; } }` — hides tab labels, keeps icons, once the modal
  gets narrow (a real scenario given the `92vw` cap on a small window).
  560px, not Armory's 767px: this bar only ever holds 4 short-ish labels
  (vs. Armory's rail holding 5, some longer — "MCP Servers"), and the
  available width per tab in a horizontal bar differs from a vertical rail's
  per-item width, so reusing Armory's number wasn't meaningful here either —
  picked to leave comfortable room for 4 icon-only tabs well before the tab
  bar would visibly wrap or truncate.

---

## 4. Verified

- `npx tsc --noEmit` — clean.
- `npm run lint:scss` — no new errors.
- `npx vitest run frontend/app/view/agent` — 669/670 passing (1 pre-existing,
  unrelated timeout flake under full-suite machine load — confirmed passing
  in isolation). No dedicated test file exists for `AgentSetupModal`.
- Not done: live interactive click-through of the narrow-width collapse.
  `task dev` was launched separately for manual testing rather than scripted
  automation, since no project skill/driver exists for interactively driving
  this native macOS CEF app (not Electron, no existing Playwright-style
  harness), and a competing dev instance risked interfering with the live
  production session this conversation itself runs inside of.

---

## 5. Blast radius (now much smaller than the first pass)

Three files: `agent-model.ts` (one-line icon change), `AgentSetupModal.tsx`
(tab icons + markup), `AgentSetupModal.scss` (container query + icon
styling). `blockframe.tsx` and `block.scss` — shared by every pane view type
— are **untouched**, unlike the reverted first attempt. Nothing outside the
agent-setup surface is affected.

---

## 6. Non-goals

- Not touching the global Armory pane's own code
  (`armory-view.tsx`/`.scss`) — reference pattern only.
- Not touching the existing failure-state "Open Armory" banner
  (`failure-accessory.ts`), which does open the *global* Armory pane — that's
  a genuinely different, unrelated affordance and was never in scope.
- Not adding a rail-to-bottom-bar layout swap to `AgentSetupModal` — its tab
  bar is already the shape Armory swaps *to* at narrow widths, so there's no
  second layout to transition into.
