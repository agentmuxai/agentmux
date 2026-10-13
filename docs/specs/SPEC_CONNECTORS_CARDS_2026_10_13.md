# Spec: Connectors shows everything connected as cards

**Status:** proposed, not built.
**Date:** 2026-10-13 · **Author:** agent2
**Trigger:** the repo owner, 2026-10-13: *"update the look of the connectors view. Just like the top section, we want cards for all the connected accounts"*, then *"we also want a similar look of cards for MCP servers and remotes .. we want a consistent style across all of connectors"*.
**Written against:** `main` @ `84e05ef4b`.
**Builds on:** `SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md` (L1 lines not fills, L2 square corners, L3 densities, L5 every state, L6 tokens only); `SPEC_RETIRE_ARMORY_CONNECTORS_AND_KNOWLEDGE_PANES_2026_10_05.md` and `SPEC_REMOTES_INTO_CONNECTORS_2026_10_08.md` (how Connectors came to have three sections); `SPEC_ARMORY_ACCOUNTS_DELETE_AND_INLINE_DETAIL_2026_10_04.md` §3.2 (details open in place, not in a modal).

---

## 1. Summary

Connectors (`view/connectors/connectors.tsx`) has three sections: **Accounts**, **MCP servers** and **Remotes**. Only the top of Accounts uses cards: the brand tiles of `AccountsGallery`. Everything that is actually connected is a row, and each section lays out its rows, its status and its details differently.

This spec makes the three sections one design:
- **One card** (`element/ui/Card.tsx`) for every connected thing: an account, an MCP server, a remote. It is styled like the brand tiles above it.
- **One grid** (`element/ui/CardGrid.tsx`) that lays the cards out, and opens a card's details **below the row the card is in**, the full width of the grid.
- **One status dot** (`element/ui/StatusDot.tsx`), replacing the three that exist today.
- **One section layout:** a header with a filter and an Add button, an Add gallery where there is something to pick from, then the cards under group headings.

Nothing changes in the data, the RPCs, what can be done to an item, or the forms. This is a layout and styling change.

## 2. What exists

| | Accounts | MCP servers | Remotes |
|---|---|---|---|
| Component | `AccountsManager` → `AccountsGallery` + `AccountsTab` (`identity/identity-accounts-tab.tsx`) | `McpManager` (`mcp/mcp-manager.tsx`) on `PrimitiveListDetail` | `RemotesView` (`remotes/remotes-view.tsx`) |
| Top | brand-tile gallery (cards), chooser OAuth / Key | "In use now" (observed, read-only rows), then a "Catalog" heading | toolbar: filter, refresh, Add remote (inline form) |
| Items | rows grouped by provider; AgentMux Cloud as a separate projected row | buttons with name and a reach badge ("All agents", "N agents"); a "Not loaded by…" note below | rows grouped Pinned / SSH hosts / Recent / WSL / Hidden: colour swatch, dot, name, platform tag, helper, sessions·agents, hover terminal/files buttons |
| Status | `.status-dot status-{valid,expired,invalid,checking,unknown}` (`identity/styles`) | `.mcp-in-use-dot` on observed agents; a live probe pill only in the detail | `.remotes-dot.is-{connected,connecting,disconnected,error}` |
| Details | panel opens under the row (`AccountDetailPanel`) | **replaces the whole list** (`PrimitiveListDetail`, back button) | panel opens under the row (`.remotes-detail`) |
| Right-click | Bind to agent, Copy ID, Delete (`buildAccountRowMenu`) | none | full menu (`rowMenu`) |
| Add | gallery tiles, `+ Add` | footer: `+ Browse catalog` (overlay picker), `+ New MCP server` | toolbar `Add remote` |
| Filter | none | none | `FilterInput` |

There is no Card, Tile or StatusDot component in `element/` or `element/ui/`. The closest is `native-memory/MemoryTile.tsx`, a generic tile the Memory pane uses.

## 3. Design

### 3.1 The section layout (all three sections)

```
┌ Accounts ·· 5 ───────────────────────── [ filter… ] [+ Add] ┐  section header
│ ┌chip┐┌chip┐┌chip┐┌chip┐┌chip┐┌chip┐                        │  Add gallery (where it applies)
│ └────┘└────┘└────┘└────┘└────┘└────┘                        │
│ CONNECTED                                                   │  group heading
│ ┌───────────────┐┌───────────────┐┌───────────────┐         │
│ │ card          ││ card (open)   ││ card          │         │  CardGrid
│ └───────────────┘└───────────────┘└───────────────┘         │
│ ┌───────────────────────────────────────────────────┐       │
│ │ details of the open card, full grid width         │       │  CardGrid detail row
│ └───────────────────────────────────────────────────┘       │
│ ┌───────────────┐┌───────────────┐                          │  the rest of the cards
└─────────────────────────────────────────────────────────────┘
```

- **Header:** the section's name and count, a `FilterInput` and the section's Add button (`Button`, accent tone). The filter is new for Accounts and MCP servers. It matches every word against the card's title and subtitle, as `matchesEveryWord` does for Remotes.
- **Add gallery:** tiles for what can be added, as Accounts has today (§3.5).
- **Groups:** a quiet uppercase heading (the existing `.accounts-connected-heading` style, moved to the grid) above each group's `CardGrid`. Empty groups aren't shown.
- **Empty section:** one line saying what goes here, with the Add button, in place of the grid.
- **One scroll container per section**, as today.

### 3.2 The card

```
┌──────────────────────────────────────┐
│ ▌[logo] Title                    ●   │   ▌ = optional colour edge (remotes)
│ ▌       Subtitle                     │
│ ▌ meta · meta              [⌨][📁]  │   hover actions, right
└──────────────────────────────────────┘
```

`Card` props: `icon` (JSX: `ProviderLogo` or a Font Awesome icon), `title`, `subtitle?`, `status?: {tone, label}`, `meta?: JSX[]`, `actions?: JSX` (shown on hover, focus and when open), `edgeColor?`, `selected`, `muted?`, `onSelect`, `onOpen?` (double-click), `onContextMenu?`, `ariaLabel`, plus `id`/`data-*` passthrough so sections keep their test hooks.

Style (L1, L2, L3, L5, L6):
- **Rest:** 1px `--border-color` line, `--button-secondary-bg-color` background, square corners. The same as `.account-tile` today.
- **Hover:** `--accent-color` line and `--hover-bg-color`, as the tile.
- **Focus:** `focus-ring` mixin.
- **Open (selected):** accent line plus a 6% accent tint (`--ui-tint-rest`). `aria-expanded="true"`.
- **Needs attention** (status tone `error`): a `--ui-line-danger` line. No fill.
- **Muted** (a hidden remote): 0.6 opacity on the content, the line unchanged.
- **Edge colour:** a 3px left edge in the given colour (a remote's `display:color`), replacing `.remotes-swatch`.
- **Density:** `ui-density-compact` on the section. Title at `--ui-font-size` + 1px and weight 600, subtitle and meta at `--text-2xs` in `--secondary-text-color`. No raw px font sizes.
- **Size:** a min width of 200px in the grid. In a pane narrower than two cards it is one column, and reads like a roomy row.
- **The whole card is the button:** `role="button"`, `tabIndex=0`, Enter/Space opens or closes it, Escape closes it. The hover actions are `IconButton`s that stop the click from reaching the card (Remotes does this today).

**The Add gallery's tiles become `Card` with `size="chip"`:** a one-line card (logo, name, status line, count badge) at the tiles' current density. The top and the cards below then share one stylesheet, and `accounts-gallery.scss` loses its own tile rules. The `.account-tile*` class names stay on the chips for the tests that key on them.

### 3.3 The status dot

`StatusDot` (`element/ui/StatusDot.tsx`): an 8px circle with `--radius-full` and `title` = its label. The tone sets the colour:

| Tone | Colour | Used for |
|---|---|---|
| `ok` | `--success-color` | account valid, remote connected, MCP server connected for every agent that reported |
| `busy` | `--warning-color`, pulsing (none under reduced motion) | account checking, remote connecting, MCP probe running |
| `warn` | `--warning-color` | account expired, MCP needs auth or not loaded by some agent |
| `error` | `--error-color` | account invalid, remote error, MCP failed or unreachable |
| `idle` | hollow ring, `--secondary-text-color` line | account unknown, remote not connected, MCP server no agent has reported |

It replaces `.status-dot status-*` in Connectors, `.mcp-in-use-dot` and `.remotes-dot`. The agent pane's Identity tab keeps its own `.status-dot` until it moves to cards too (out of scope, §6).

### 3.4 Details: below the card's row

`CardGrid` props: `items`, `key(item)`, `renderCard(item, {selected})`, `selectedKey`, `renderDetail(item)`, `minCardWidth` (default 200).

When a card is open, the grid inserts a full-width detail row (`grid-column: 1 / -1`) **after the last card of the open card's row**, not straight after the card, so the cards in that row stay together. The grid measures its own column count with a `ResizeObserver` (`floor((width + gap) / (minCardWidth + gap))`) and places the detail row accordingly. The row moves when the pane is resized.

- The detail row has a small caret pointing at the open card (`left` from the card's column) so it reads as that card's details.
- One card is open per grid. Opening another moves the panel; clicking the open card closes it.
- It scrolls into view the way `AccountDetailPanel` does today: its bottom shows without the card's top leaving the view (`findScrollParent`, no `scrollIntoView`).
- Escape inside the panel closes it and returns focus to the card.

**MCP servers changes most here:** its details stop replacing the list (`PrimitiveListDetail`) and open below the card, like the other two sections. The edit and new forms open in the same place: a new server's form opens as the detail row at the top of the grid.

### 3.5 Per section

**Accounts** (`AccountsManager`):
- **Add gallery:** the brand tiles, unchanged in behaviour (chooser, AgentMux panel, Claude login panel), rendered as chip cards.
- **One group, "Connected"**, flat: the logo already says the provider, and the gallery above already counts accounts per provider. AgentMux Cloud's card comes first, then accounts in the gallery's provider order, then by label.
- **Card:**
  - icon: `ProviderLogo` of the account's provider;
  - title: `accountLabel` (the login email, else the name; the generic name stays in the tooltip);
  - subtitle: the display name if set, otherwise the provider's brand label;
  - meta: the kind ("OAuth", "API key", "Token", "IAM role", "Env var": short forms of `KIND_LABELS`) and "Used by N agents";
  - status: `account.status`;
  - right-click: `buildAccountRowMenu`, unchanged.
- **The AgentMux Cloud card:** title is the session's email (else "AgentMux Cloud"), subtitle "AgentMux Cloud", status from `muxbus.status()` ("sign in again" is `warn`). It opens `AgentMuxConnectPanel` as today, not a detail row.
- **Details:** `AccountDetailPanel`, unchanged inside.
- **"Used by N agents"** needs the agent links for every card, not only the open one. Fetch `ListAllAgentIdentitiesCommand` once per section and refresh it on `identityaccounts:changed`; don't fetch per card. `useAccountAgentNames` takes the shared result.
- The delete notice and `DeleteAccountConfirm` are unchanged.
- **`AccountsTab` stays** for the agent pane's Identity tab, which shares its row classes. Connectors stops using it.

**MCP servers** (`McpManager`):
- **Add gallery:** a chip per preset in `MCP_PRELOAD_CATALOG` (Ableton Live, TouchDesigner), and a "Custom server" chip. This replaces the footer's `+ Browse catalog` / `+ New MCP server` and the overlay picker `McpCatalogPicker`. A preset chip that's already configured shows "Added" and opens that server's card.
- **Two groups:**
  - **"Your servers":** the catalog (`McpServerCatalogItem`).
    - icon: the preset's logo if the name matches one, else `fa-plug`;
    - title: name;
    - subtitle: the transport and the command or URL from `config`, shortened;
    - meta: reach ("All agents" / "N agents") and "Containers too" when set;
    - status, from what agents report (`McpObservedModel`), not a probe per card:
      - `ok` if every agent that reports it is connected;
      - `warn` if any needs auth, or the not-loaded note applies (its text goes in the tooltip, and the note line moves into the details);
      - `error` if any failed;
      - `idle` if none reported.
    - The live probe (`watchMcpCapability`, every 4 s) runs only while a card is open, as today.
  - **"Also in use":** observed servers that aren't in the catalog: built in, claude.ai connectors, account and project config.
    - title: name; subtitle: `sourceLabel`; meta: `rowSummary` ("2 agents, 1 not connected"); status as above.
    - Details: the per-agent list `McpInUse` shows today (dot, agent, status, tools). Read-only, with no edit or delete.
- **Details** (catalog server), the current detail view moved below the card:
  - the reach Switches, the status pill (as a `StatusDot` with the same labels), transport and config, Bind to agent, Edit, Delete;
  - the remediation and risk notes.
- **Delete gets a `ConfirmModal`**, naming the agents it's bound to. Today it deletes on one click.
- **New: right-click on a catalog card:** Edit, Bind to agent ▸, Delete…. Observed cards have no menu.
- **No longer used by MCP:** `PrimitiveListDetail` and the `.agent-primitive-modal-*` classes. They stay for Skills, Bundles and the agent pane's modals, which share them.

**Remotes** (`RemotesView`):
- **No Add gallery.** Remotes are discovered (ssh config, WSL, recent), and adding is one free-form destination, so the header's `Add remote` opens the existing inline form as the first detail row.
- **Groups unchanged:** Pinned, SSH hosts, Recent, WSL, and Hidden collapsed behind its toggle (`groupRemotes`).
- **Card:**
  - icon: `fa-terminal` for ssh, the Linux penguin for wsl;
  - title: `displayName`; subtitle: the real name when a nickname is set (`.remotes-realname`) and the platform ("Linux · x86_64");
  - meta: the helper state and "N sessions · N agents";
  - edge colour: `display:color`;
  - status: `record.status.state`, with the error text in the tooltip;
  - muted when hidden;
  - hover actions: New terminal, Browse files;
  - double-click: New terminal;
  - right-click: `rowMenu`, unchanged.
- **Details:** `.remotes-detail` moved below the card's row. Its raw `<button>`s become `Button`, and the colour radios become `ColorSwatchPalette` (`components/color-swatch-palette.tsx`). The UI-primitives ratchet then goes down: hand-rolled buttons, and `.remotes-tag`'s 8px radius.
- **Kept:** `data-remote`, `.remotes-dot` (now on the `StatusDot`), `.remotes-realname`, `.remotes-section-heading`, and every accessible name the tests use (§5).
- **Kept:** `remotes:expand` from another pane opens that card.

## 4. Changes by file

| File | Change |
|---|---|
| `element/ui/Card.tsx`, `CardGrid.tsx`, `StatusDot.tsx`, `card.scss` | New (§3.2–3.4); exported from `element/ui/index.ts` |
| `view/accounts/accounts-manager.tsx` | Section header and filter; `CardGrid` of account cards and the AgentMux card; stops using `AccountsTab` |
| `view/accounts/AccountsGallery.tsx`, `accounts-gallery.scss` | Tiles render as chip `Card`s; tile rules move to `card.scss` |
| `view/accounts/account-card.tsx` | New: the account card and the AgentMux Cloud card |
| `view/identity/identity-accounts-tab.tsx` | Export `AccountDetailPanel` and `DeleteAccountConfirm`; `useAccountAgentNames` accepts a shared link list |
| `view/mcp/mcp-manager.tsx` | Rewritten on `CardGrid`; Add gallery; two groups; details and forms below the card; confirm on delete; right-click menu |
| `view/mcp/mcp-in-use.tsx` | Becomes the observed cards and their details |
| `view/mcp/McpCatalogPicker.tsx`, `mcp-catalog-picker.scss` | Removed (replaced by the Add gallery) |
| `view/remotes/remotes-view.tsx`, `remotes-view.scss` | Rows become cards in `CardGrid`; details below the row; `Button` and `ColorSwatchPalette` in the details |
| `scripts/ui-screenshots/memory-shots.mjs` | MCP selectors (`.agent-primitive-modal-list-item`, "New MCP server") updated to the cards |

No RPC, srv, data or meta change.

## 5. Tests

- **`Card` and `StatusDot`** (`element/ui/ui.test.tsx`):
  - role, `aria-expanded`, Enter/Space/Escape;
  - hover actions don't open the card;
  - double-click calls `onOpen`;
  - right-click calls `onContextMenu`;
  - the tone sets the class and the label sets the title.
- **`CardGrid`:**
  - with a fixed width (a stubbed `ResizeObserver`), the detail row comes after the last card of the open card's row, for the first, middle and last card of a row and for a short last row;
  - it moves when the width changes;
  - one card is open at a time.
- **Accounts** (new `accounts-manager.test.tsx`):
  - AgentMux first, then provider order;
  - the filter;
  - "Used by N agents" from one shared fetch (the RPC is called once for N cards);
  - opening a card shows `AccountDetailPanel`, and right-click builds the account menu.
  - `ClaudeLoginPanel.test.tsx` and the gallery tests pass unchanged (the `.account-tile*` hooks are kept).
- **MCP** (new `mcp-manager.test.tsx`, its first DOM test):
  - preset chips, with "Added" for one that exists;
  - the two groups;
  - the card's status from observed data (all four tones);
  - details below the card and the probe watched only while open;
  - delete asks first.
  - `mcp-model.test.ts` and `mcp-observed.test.ts` pass unchanged.
- **Remotes:** `remotes-view.test.tsx` passes with only selector changes where a row became a card. Its accessible names, `data-remote` and the section headings are kept on purpose. `remotes-sections.test.ts`, `add-remote.test.ts` and `open-remotes.test.ts` pass unchanged.
- **`section-pane.test.tsx`:** unchanged (it mocks the three sections).
- **Live,** in an isolated `task dev` build, with screenshots before and after for each section:
  - narrow and wide panes;
  - open a card in the first, middle and last column;
  - resize with a card open;
  - keyboard only;
  - light and dark themes;
  - zoom.

## 6. Out of scope

- The agent pane's Identity tab (`AccountsTab` rows, `.status-dot`). It can adopt `Card` later.
- The forms themselves (`AccountForm`, the MCP form, the Add remote form), apart from where they open.
- Memory's `MemoryTile`, the Agent picker cards, and the other tile styles listed in the research notes. Moving them onto `Card` is a separate cleanup once `Card` exists.

## 7. Phases

1. **`Card`, `CardGrid`, `StatusDot`, and Accounts:** the gallery chips and the connected cards. One PR.
2. **MCP servers.** One PR.
3. **Remotes.** One PR.

Each PR ships with screenshots and a changeset. Phase 1 settles the look, and 2 and 3 follow it.

## 8. Open questions

1. **Accounts: flat or grouped by provider?** Proposed flat, since the logo and the gallery's counts already show the provider. With many accounts, grouping by provider (as the rows are today) is easy to switch on in `CardGrid`.
2. **Details below the row, or in a side panel?** Proposed below the row: it matches what Accounts and Remotes do today and works in a narrow pane. A side panel would suit wide panes better, but needs a second layout for narrow ones.
3. **MCP: keep "Also in use" as its own group, or fold observed status into the catalog cards only?** Proposed: both. Catalog cards show their observed status, and "Also in use" lists only what isn't in the catalog, so nothing is shown twice.
4. **Remotes: an Add gallery?** Proposed none, since remotes are discovered and adding is one destination. One alternative is chips for "SSH host" and "WSL distribution" if WSL ever needs adding by hand.
5. **Card width.** 200px minimum fits two cards in a half-width pane at default zoom. Check it in the live pass and adjust.
