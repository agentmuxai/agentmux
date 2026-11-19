# Spec: Connectors is one list, one connector per line, details on click, open at startup

**Status:** active — approved by the operator 2026-11-19; the list of cards, the reflow, details in place and the no-success-step flash (§4.1 to §4.7, §4.9) shipped in PR #7812 (task #48120); §4.8 (open at startup) is not started. Verified 2026-11-19.
**Date:** 2026-11-19
**Author:** Naki-2@narko
**Verified against:** `agentmux` `main` @ `933c41a9fa` (code read 2026-11-19) and the vendor pages in §5, fetched 2026-11-19.
**Builds on:** `SPEC_CONNECTORS_CARDS_2026_10_13.md` (the cards and the in-place details this spec keeps), `SPEC_CONNECTORS_EVERY_WAY_TO_CONNECT_2026_11_14.md` (the "every way to connect" guide data this spec keeps), `SPEC_REMOTES_INTO_CONNECTORS_2026_10_08.md`, `SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md`, the `ui-confirm-panels` skill (no window modals).
**Pictures:** `SPEC_CONNECTORS_ONE_LIST_2026_11_19_mocks/` (three, at 280, 700 and 1600 px; HTML sources next to the PNGs). They show intent, not built UI.

**Changed since the version the operator read (revision 2):**
1. Each connector is a **card** and the cards reflow with the pane's width (one column at 280 px and under, two to four columns as it widens, at most four and at most 380 px each); this reverses "no tiles, no grid" and "the list never becomes a grid" (§4.1, §4.2, §4.7).
2. An opened card's details keep the full-width panel under its row; §4.7 now says how focus, reading order and a changing column count follow, and the order of the cards never depends on the column count.
3. D2 is checked again against cards and **still stands** (§3); the pictures are redrawn at three widths.

---

## 1. Ask

Stated as requirements:

1. Connectors opens with the app: it is part of the default arrangement, with the panes that open today.
2. Connectors reads as **one list, one card per connector**: an icon, a name, one plain line saying what it is for, and one clear control. A connector that is not connected says **Connect**. The cards reflow with the pane's width, from a thin pane to a very wide one.
3. **A connected connector opens its whole connection in place when clicked**: every way it is connected, every account (the "instances"), and what uses it. This replaces the two separate lists the Accounts section has today.
4. Research first: how other agent harnesses and assistants present connectors and tools, with sources (§5).

The layout reference was a plain list of eight rows (icon, name, one line, a Connect button at the right, and a "browse all" line at the foot). It is a reference for the layout only; AgentMux keeps its own look, words and icons. The operator then asked that the one list work in thin and very wide panes, so each row is a card that reflows (§4.7).

## 2. What exists today (read in code)

**The pane.** `view/connectors/connectors-instance.tsx` is a `SectionPane` with three sections (`store/section-panes.ts`): **Accounts** (default), **MCP servers**, **Remotes**.

**The two lists in Accounts** (`accounts/accounts-manager.tsx`):

| List | Component | What it is |
|---|---|---|
| 1. Add gallery | `AccountsGallery` over `SERVICE_CATALOG` (`accounts-catalog.ts`) | A grid of 11 chips (AgentMux, GitHub, Google, AWS, OpenAI, Anthropic, xAI, Moonshot, Slack, Custom). A chip shows its count of connected accounts. Clicking opens `AddAccountSection` under it: how to connect (sign-in or key), then the sign-in or form, in place. xAI and Moonshot are text only (they sign in inside the agent). "Other agents" (`OtherAgentsSection`) sits under the grid: one line each on how the agents without a tile sign in. |
| 2. Connected | `CardGrid` of `AccountCard`s under a "Connected" heading | One card per stored account, plus AgentMux Cloud's session as a projected card. Clicking opens `AccountDetailPanel` under it (status, fields, `OAuthAccountSection`, `CliSignInSection`, who uses it, Reconnect, Edit, Delete). Right-click: bind to agent, copy ID, delete. |

So a person finds a service in one list to add it and in another to manage it. That is the pair this spec removes.

**MCP servers** (`mcp/mcp-manager-view.tsx`) has the same shape: an Add gallery (`MCP_PRELOAD_CATALOG` presets and "Custom server"), then two card groups, "Your servers" and "Also in use" (observed, read-only). **Remotes** (`remotes/remotes-view.tsx`) lists SSH hosts and WSL distributions; it is not a sign-in to a service.

**Inside an account's details** a service can have more than one account (GitHub, Google), more than one way in (GitHub's App or a token; Anthropic's and OpenAI's sign-in or key), and extra parts: a Google account's services (Gmail, Calendar, Drive: state, Turn on, who may use them and at what level; `GoogleServicesSection`), a GitHub account's installations (`GithubInstallationsSection`), and the agents bound to the account.

**What opens at startup today:** `agent | [sysinfo / swarm]`, from three places that must agree by hand:

1. First launch: `default_three_pane_tree` in `crates/srv-core/src/backend/wcore/mod.rs`.
2. A new window: `crates/srv/src/server/service/window_create.rs` (it seeds three views and calls the same function; it only treats a seed of exactly three blocks as the default).
3. A new tab: `DEFAULT_TAB_PRESET` in `frontend/app/tab/tab-presets.ts`. Its comment says Connectors and Memory are left out of the starter set on purpose; this spec reverses that for Connectors.

`defwidget@connectors` already exists (`crates/srv-core/src/config/widgets.json`).

## 3. Decisions for the operator

Each has my recommendation; the rest of the spec is written to it.

| # | Question | Recommendation |
|---|---|---|
| D1 | What does "open at startup" cover? | The **default arrangement**: first launch, every new window, every new tab. A saved layout is not rearranged, and a pane the person closed stays closed. It does not reopen Connectors on every launch. |
| D2 | Where does it sit? | **Still a third column on the right**: `agent (5) | [sysinfo / swarm] (3) | connectors (2)`. Checked again with cards: at 2 of 10 on a 1920 px window the column is about 380 px, one column of cards that reads like the list; on a 1366 px window it is about 270 px, the thin layout, so thin is the everyday case on a small screen and is designed first (§4.7). A person who maximises or widens the pane gets more columns of cards. Stacking it under swarm would make a short, wide strip and squeeze swarm, so it is not recommended. Sizes are tuned on the debug build you try before merge. |
| D3 | What are "instances"? | Each **account** of a connector (two GitHub accounts, two Google accounts), with which agents use it. If you meant something else (a remote machine, a running server), say so. |
| D4 | Do MCP servers join the list? | **Yes, as cards** ("Tool server" in the one-line description). The two MCP groups go away with the Accounts gallery. **Remotes stays its own section**: a machine you reach is not an account you sign in to. |
| D5 | Google: one card or three? | **One card** ("Google"). Gmail, Calendar and Drive share one account and one sign-in, and each is turned on inside the opened card. Three cards would show three Connect buttons that all start the same sign-in. |
| D6 | Order | **Connected first, then Available**, each in a fixed order (AgentMux Cloud first, then the catalog's order). Not by use, so cards don't reshuffle. |

## 4. Design

### 4.1 One list

The Accounts section becomes the list; its section id stays `accounts` (saved panes, `openConnectors("accounts")` and the agent-notice link keep working). The section bar then reads **Connectors** (the list) and **Remotes**; a saved `mcp` section opens the list with its tool-server cards shown.

```
thin (280)            medium (700)                       very wide (1600)
┌ Connectors ┐        ┌ Connectors ───────── [Filter…] ┐  ┌ Connectors ──────────────────────── [Filter…] ┐
│ CONNECTED  │        │ CONNECTED                      │  │ CONNECTED                                     │
│ ┌────────┐ │        │ ┌──────────┐ ┌──────────┐      │  │ ┌──────┐ ┌──────┐                             │
│ │ card   │ │        │ │ card     │ │ card     │      │  │ │ card │ │ card │                             │
│ └────────┘ │        │ └──────────┘ └──────────┘      │  │ └──────┘ └──────┘                             │
│ AVAILABLE  │        │ AVAILABLE                      │  │ AVAILABLE                                     │
│ ┌────────┐ │        │ ┌──────────┐ ┌──────────┐      │  │ ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐           │
│ │ card   │ │        │ │ card     │ │ card     │      │  │ │ card │ │ card │ │ card │ │ card │           │
│ └────────┘ │        │ └──────────┘ └──────────┘      │  │ └──────┘ └──────┘ └──────┘ └──────┘           │
└────────────┘        └────────────────────────────────┘  └───────────────────────────────────────────────┘
```

- **One collection of cards.** There is no add gallery, no second list and no row of chips: the chips gallery and the Connected card grid are deleted from Connectors. Two quiet headings, **Connected** and **Available**, sit over one stream of cards in one order (§4.7); a card moves from Available to Connected when it connects.
- **A header** with the count and the existing `FilterInput` (kept from the cards spec), no Add button: Connect is on the cards, and "Add a custom connector" at the foot opens the Custom key form and the custom tool-server form (today's `Custom` tile and `Custom server` chip).
- **Not a "Browse all" page.** The catalog today is 11 service entries and two tool-server presets, which fit one collection. If it grows past about 20 cards, the foot line becomes a "Browse all" that opens the same cards unfiltered with a search box; nothing in the design blocks it.

### 4.2 A card

`icon · name · one line · one control`: the existing `Card` (`element/ui/Card.tsx`) in a `CardGrid`, which the cards spec already built for Connectors.

- **Icon:** `ProviderLogo` (as now).
- **Name:** the connector's name.
- **One line:** a not-connected card says what the connector is for, in the words of the 11-14 guide data (`provider-guide.ts`) where it has them; at most two lines, then an ellipsis with the full text on hover and in the opened details. A connected card's line is its state (§4.3).
- **One control:** **Connect** when not connected; **›** when connected (the whole card is the disclosure); **Reconnect** replaces **›** when the card needs a sign-in again, and the card still opens on click.
- **Two layouts, chosen by the card's own width** (a container query on the card, not on the window or the column count): at 340 px and wider, a row (icon, text, control at the right); under 340 px, stacked (icon and name, then the line, then the control under the text, left-aligned). So a card never changes shape just because a neighbour did.
- **Equal height within a row of cards:** the grid stretches cards to the tallest in their row; the control stays at the bottom edge of the card in the stacked layout.
- **Text-only connectors** (xAI, Moonshot, and the other agents of today's "Other agents" list) have no Connect: their control is **How to connect**, which opens the guide text in the card's details. They are the end of the Available group, under an "Agents that sign in inside their own pane" heading, so no card promises a button that does nothing.
- **Right-click** keeps today's account menu on the account lines inside an opened card.

### 4.3 State

A card shows its state as a word, never a coloured dot alone (the status dot stays beside the word):

| State | Word | Control | Source |
|---|---|---|---|
| Not connected | none (the one-line "what it is for") | Connect | no account for the brand |
| Connecting | "Connecting…" | Cancel | the sign-in or form that the card's opened section runs |
| Connected | "Connected · <email or account label>", or "N accounts" | › | accounts with status `valid` |
| Needs sign-in again | "Needs sign-in again" (warn tone) | Reconnect | status `expired`; `canReconnect` |
| Failed | "Couldn't connect: <reason>" (error tone) | Reconnect | status `invalid`, and the error text the account already carries |
| Several accounts, mixed | "2 accounts · 1 needs sign-in again" | › | the worst state wins the tone; the words say both |

"Connected on another machine" is **not a state**: credentials are kept per computer today, so another machine's connection is not visible here. I do not invent it; if the operator wants it, it is its own task (it needs AgentMux Cloud to say what a signed-in machine has).

The state words come from `oauth-account-state.ts` and `account-card.tsx` (which already compute them), and the failed reason is the account's own text, trimmed to one line with the full text in the details.

### 4.4 What Connect does, per kind

Connect never opens a window. It opens a section **inside the card**, in the details panel under the card's row of cards (the caret-topped section the add flow already uses), and closes it when the account is saved or the person cancels.

| Kind | Connectors | Connect opens |
|---|---|---|
| Browser sign-in, one way | Google, Slack | The sign-in runs: "Open the browser" and the wait, as `OAuthConnectPanel` shows today. |
| AgentMux Cloud | AgentMux Cloud | `AgentMuxConnectBody` (sign-in choices) as now. |
| A choice of ways | GitHub (App or token), Anthropic and OpenAI (sign-in or key) | The chooser of ways, each with the guide's one-line explanation (`AddAccountSection` today), then the sign-in or the form. |
| Key form | AWS, Custom | The Vault key form, as the Add account form does. |
| Tool server | Ableton Live, TouchDesigner, custom | `Add` (not Connect): the preset's form, as the MCP chip does. |

This is the same code that runs today under a chip. The change is where it is reached from: the card, not a gallery.

### 4.5 A connected card, opened

Clicking a connected card opens its details in a panel the full width of the grid, **under the row of cards the open card is in** (the cards spec's §3.4 behaviour, kept), with a caret under the open card and one card open at a time. How it follows the column count, focus and reading order is §4.7. Contents, top to bottom; a part with nothing to say is not drawn:

1. **Accounts (the instances).** One line per account: label, how it is connected ("AgentMux GitHub App", "Personal access token", "ChatGPT sign-in"), its state word, "used by N agents". One account: its full details (today's `AccountDetailPanel`) show directly; more than one: each line opens its own details. Per account: **Reconnect**, Edit, Bind to agent, **Disconnect…** (the delete confirm opens in the details, with the agents it names).
2. **Parts the connector has:** a Google account's services and who may use them; a GitHub account's installations; CLI sign-in status. These are today's `GoogleServicesSection`, `GithubInstallationsSection` and `CliSignInSection`, moved, not rewritten.
3. **Other ways to connect:** the ways the connector offers that this account did not use (a token for GitHub, a key for Anthropic), each with its one-line explanation and an **Add** button. This is the 11-14 spec's "every way to connect", kept.
4. **Add another account** (the same Connect section, opened here).
5. **What it can do and can't** (one or two lines, from the guide data), where the guide has them.

### 4.6 Nothing a person can do today is lost

| Today | In the one list of cards |
|---|---|
| Chip → add an account (sign-in or key) | Connect on the card, §4.4 |
| A chip's "N connected" count | The card's "N accounts" line |
| Connected card, status dot and word | The card's state word, §4.3 |
| Card → account details, Reconnect, Edit, Delete confirm | The opened card, §4.5.1 |
| Right-click: bind to agent, copy ID, delete | Same menu, on the account lines |
| AgentMux Cloud card, sign-in choices, Sign out | The AgentMux Cloud card and its opened section |
| Google services, grants, call log; GitHub installations | The opened card, §4.5.2 |
| Filter | Header filter, over cards |
| "Other agents" lines and docs links | The "Agents that sign in inside their own pane" group at the foot |
| xAI and Moonshot text tiles | Their cards, control **How to connect** |
| MCP: Add presets, Custom server | Cards with **Add**, and "Add a custom connector" at the foot |
| MCP: "Your servers" cards, reach switches, probe, Edit, Delete, Bind | Connected tool-server cards, opened |
| MCP: "Also in use" (observed, read-only) | Connected tool-server cards whose details are read-only |
| `remotes:expand`, Remotes list | Unchanged (Remotes section) |
| A refusal notice's "Open Connectors" link | Opens this list (`accounts`) |
| `requestReconnect(accountId)` from elsewhere | Opens the card and runs Reconnect |

### 4.7 Thin to very wide, order, focus and reading order

**What measures the width.** The Connectors pane's own body, never the window: the body is a named container (`container: connectors / inline-size`), and `CardGrid` already measures its own element with a `ResizeObserver` (`columnsFor`), so a pane dragged narrower or wider reflows the cards, whatever the window does. The grid is `repeat(auto-fill, minmax(270px, 1fr))`, gap `--space-1-5` (6 px), pane padding 10 px, with two limits added for this pane (two optional `CardGrid` props, `maxColumns` and `maxCardWidth`): **at most four columns, each card at most 380 px.**

**The steps.** The widths are what 270 px minimum, 6 px gap and 10 px padding give; they are tuned on the build:

| Pane width | Columns | Card width | Card layout (by the card's own width, §4.2) | Picture |
|---|---|---|---|---|
| under about 340 px (down to 280, and under) | 1 | the pane | stacked: control under the text | 1 |
| about 340 to 565 px | 1 | 320 to 545 | stacked under 340, row from 340 | |
| about 566 to 841 px | 2 | 270 to 410 | stacked to 339, row above | 2 (700 px) |
| about 842 to 1117 px | 3 | 270 to 360 | stacked to 339, row above | |
| 1118 px and wider | 4 (the most) | 270 up to the 380 cap | row from about 1398 px | 3 (1600 px) |

- **Thin (280 px and under).** One column, stacked cards: icon and name, then the line (two lines at most), then the control under the text. The name and line wrap; nothing scrolls sideways. Below 280 the same layout holds down to the pane's minimum, and the pane keeps vertical scrolling only. This is designed first: a column of 2 of 10 on a 1366 px window is about 270 px (§3, D2).
- **Very wide.** Four columns at most and 380 px at most per card, so cards never stretch into long bars and an ultrawide pane never shows nine or ten tiny columns to scan. The header and the grid share one block, at most about 1560 px wide, left-aligned under the pane's title; the rest of an ultrawide pane stays empty, as a settings page does. (Two constants, `maxColumns` and `maxCardWidth`, set the limits: a different taste is a one-line change.) The opened card's details take the full width of that block; their content is one column up to about 720 px and two columns (accounts on the left, parts and other ways on the right) from about 900 px, so a line never runs longer than a readable measure (picture 3).
- **Equal rows.** Cards in a row of cards are the same height; the control sits at the same place in each.

**Order never depends on the column count.** The order is data: Connected first (AgentMux Cloud, then catalog order), then Available (catalog order), then the text-only agents. The grid fills row by row, left to right, as the document reads; nothing is placed by column or packed to fill gaps (no masonry, no `dense` packing). A resize changes only where lines wrap. A card that connects moves between the two groups, not within one.

**Where the details go, and what follows.**
- **Placement holds:** the panel goes after the last card of the open card's row, full width of the grid, with the caret under the card (`caretLeft` is already computed from the card's column). This still reads as that card's details in two, three or four columns, and the cards of the row stay together. In one column it is simply under the card.
- **A changing column count moves the panel, not the choice:** when a resize changes the number of columns, the open card stays open, the panel is placed after the end of the card's new row, the caret follows, and the card is kept in view (the existing reveal runs again on a column change, which today it only does on opening). Focus is not lost, because cards are keyed by connector.
- **Focus order:** opening a card moves focus to the first control in its details. Tab then continues through the details and on to the next row of cards. Esc anywhere in the panel closes it and returns focus to the card (`CardGrid`'s `closeFromDetail`, kept). Closing a card by clicking it again keeps focus on it.
- **Reading order:** the document holds the cards of the row, then the details, then the next row. So the details are a `role="region"` labelled by the open card's name ("GitHub details"), and the open card carries `aria-expanded="true"` and `aria-controls` pointing at it, so a screen-reader user is told where the details are even though a card or two follow the open one in the DOM. Putting the panel right after the open card and letting the grid pack the next card up beside it (`grid-auto-flow: dense`) was considered and rejected: it makes Tab jump upward visually to the cards that were packed back into the row.

**Keyboard and screen reader (any width).**
- Tab reaches each card's disclosure, then its control; Enter or Space opens or closes it; arrow keys move between cards in the grid's visual order (left and right within a row, up and down between rows).
- Each card is a list item holding a disclosure button (name and state word as its label) and, separately, the control button ("Connect Google"); the control is never nested inside the disclosure. State is a word, never only a colour. A change of state (Connecting → Connected) is announced once (`role="status"`).
- **No window modals.** Every form, chooser and confirm opens inside the card's details.

### 4.8 Open at startup

Per decision D1 and D2. The three places in §2 change together:

- `default_three_pane_tree` takes a fourth block and returns the three-column tree; its callers (first-launch seed, `window_create`) seed four views (`agent`, `swarm`, `sysinfo`, `connectors`). `window_create`'s "exactly three seeded blocks is the default" check and `remember_seeded_default` become four.
- `DEFAULT_TAB_PRESET` gains `defwidget@connectors`, and its comment is rewritten.
- The Rust tests that pin the tree (`server/tests.rs`, `tests/integration_test.rs`, `wcore`) are updated. This is a Rust change, so Opaz runs the tests.
- A saved layout is read as saved. No migration.

The new pane starts on the list (`connectors:section` unset means `accounts`).

### 4.9 No success step: the card flashes "Connected"

Added after approval, from the operator's words relayed by AgentY on 2026-11-19: no success step after "Connected", in the agent pane and in Connectors; "we can tell from the UI"; in Connectors, flash "connected" inside the pane.

- **When a connect succeeds, nothing is left to read and dismiss.** No modal, no "Signed in" panel with Done or OK. The Connect section collapses at once (a sign-in that worked, or an account saved).
- **The card flashes, once, in place.** The card is now in Connected; for about 2.8 seconds it holds a success-coloured line and tint with "✓ Connected" where the chevron goes, then goes back to its connected state. The state word under the name ("Connected · <account>") is there throughout and stays.
- **It shows with animation effects off.** The flash is a held state that a timer removes, not a keyframe animation, so the OS's reduced-motion setting does not switch it off; the preference only removes any fade back.
- **Screen readers hear it once:** a `role="status"` line ("OpenAI connected") is added to the card for the flash; the visible badge is hidden from them.
- **Only a connect the person started here flashes.** A card flashes when its Connect section was opened in this pane and the connector then becomes connected. The first load of the list, where every connected card appears at once, flashes nothing.
- **A failure still shows its reason** in the section, with Retry, as today. Only the success step is gone.
- **The agent pane side** (the in-pane sign-in panel closing by itself on success) is #48130, Agent1's. It is one shared component (`SignedIn` in `provider-login-view.tsx`), so that task removes it; Connectors does not depend on it, because its own sections close the panel as soon as the sign-in reports success.

## 5. Research: what other harnesses do

Read 2026-11-19, each page fetched. Practices are paraphrased. **Not found:** the ChatGPT help pages returned 403 to every fetch, so nothing here claims what ChatGPT does; and I found no documented guidance on narrow layouts or screen readers for a connector list in any product, so §4.7 is our own rules, from the UI spec.

### Practices to adopt

1. **A status word with the reason, and the fix in the row.** Claude Code's `/mcp` shows Connected, Needs authentication, Failed to connect, Pending approval or Disabled, and offers Reconnect, Re-authenticate and Clear authentication on the server's menu. <https://code.claude.com/docs/en/mcp> → §4.3, §4.5.
2. **"Needs sign-in again" is its own state, on the list and on the detail.** Windsurf's (Devin Desktop's) MCP page says an expired OAuth credential shows "Needs authentication" in the list, on the card and on the detail page, and Authenticate clears the stored credentials and reruns the browser sign-in. <https://docs.devin.ai/desktop/cascade/mcp> → §4.3 (Reconnect replaces the chevron).
3. **Separate "available to the agent" from "approved to run".** VS Code lets a tool be turned off (the agent never sees it) apart from approving each call, and keeps trust decisions apart from tool approvals. <https://code.visualstudio.com/docs/agents/run/tools> and <https://code.visualstudio.com/docs/agents/run/manage-mcp-servers> → the opened row keeps "used by N agents" (who has it) apart from a level control (what they may do, as Google's grants have today).
4. **Per-tool permission as three choices, grouped by read-only against write or delete.** Claude's connectors page lets an owner pick Always allow, Needs approval or Blocked per category or tool, with tools grouped as read-only or write/delete; the page names no default, and these owner settings apply to the whole organisation. Raycast approves read-only tools without asking and asks for the rest; Zed defaults to confirm. <https://support.claude.com/en/articles/11176164-use-connectors-to-extend-claude-s-capabilities>, <https://manual.raycast.com/ai/ai-extensions>, <https://zed.dev/docs/ai/tool-permissions> → adopt as the shape of any future per-tool control; **not built here** (§7).
5. **Describe what it can and can't do.** Gemini's app page shows the description, the actions the app supports and doesn't support, and example prompts; Disconnect is in a menu on the app. Claude's directory labels entries Verified or Community and says Verified is not an audit. <https://support.google.com/gemini/answer/13695044>, <https://claude.com/docs/connectors/directory> → §4.5.5.
6. **One connected list, with a separate discover view for a long catalog.** Claude's directory has a Discover tab where Connect starts the sign-in. The page says nothing about search or ordering, so "connected first" is our choice, not theirs. <https://support.claude.com/en/articles/14328846-browse-skills-connectors-and-plugins-in-one-directory> → §4.1.
7. **Keep who connected it apart from who allowed it.** On Team plans a member who cannot enable a connector sees a Request button, and each member still signs in with their own account. <https://claude.com/docs/connectors/directory> → the details name the account (whose) and, later, the grant (who allowed).

### Practices not to adopt

1. **Remove and add again as the only way to change or repair a connector.** Claude's custom connectors are changed by removing and re-adding them, and a re-add without a remove leaves two. <https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp> → we keep Reconnect, Edit and Add another.
2. **Raw rule names or a config file as the permission screen.** Claude Code writes tools as `mcp__<server>__<tool>` and Zed as `mcp:<server>:<tool>` in settings, and VS Code shows some servers as read-only JSON. <https://code.claude.com/docs/en/permissions>, <https://zed.dev/docs/ai/mcp>, <https://code.visualstudio.com/docs/agents/run/manage-mcp-servers> → plain names and controls.
3. **A coloured dot with a hover tooltip as the only status.** Zed shows a dot and a tooltip. A hover is unavailable at 280 px, to a keyboard and to a screen reader (this last point is my inference, not the page's). <https://zed.dev/docs/ai/mcp> → a word beside any dot (§4.3).

## 6. Changes by file

| File | Change |
|---|---|
| `view/connectors/connector-entries.ts` (new) | Cards from `SERVICE_CATALOG`, `MCP_PRELOAD_CATALOG`, the accounts, the AgentMux Cloud session and the observed MCP servers: name, line, state word, control, order (§4.2, §4.3). Pure, tested. |
| `view/connectors/connectors-list.tsx`, `connector-card.tsx`, `connector-details.tsx`, `connector-card.scss` (new) | The collection, the card (two layouts by the card's own width), the pane as a named container. |
| `element/ui/CardGrid.tsx`, `card.scss` | Two optional props, `maxColumns` and `maxCardWidth`; the reveal runs again when the column count changes; the details panel is a labelled region and focus moves into it on open. Other users of `CardGrid` are unchanged. |
| `view/accounts/accounts-manager.tsx` | Renders the list; the gallery and Connected grid go. |
| `view/accounts/AccountsGallery.tsx`, `accounts-gallery.scss` | Deleted (the add flow moves to `connector-card`); `AddAccountSection` is kept and opened by Connect. |
| `view/accounts/OtherAgentsSection.tsx` | Becomes the foot group. |
| `view/mcp/mcp-manager*.tsx` | Cards join the list (phase 2); `McpManager`'s own page and gallery go. |
| `store/section-panes.ts` | Sections `accounts` (label "Connectors") and `remotes`; `mcp` accepted and mapped. |
| `crates/srv-core/src/backend/wcore/mod.rs`, `crates/srv/src/server/service/window_create.rs`, `session_other_builds.rs`, the tests that pin the tree | The fourth pane (§4.8). |
| `frontend/app/tab/tab-presets.ts` | `DEFAULT_TAB_PRESET` and its comment. |
| `frontend/app/i18n/locales/en.json` | Keys for the new words. |

## 7. Out of scope

- **Per-tool permissions** (always allow, ask, never). Practice 4 is where it should go; it needs a place to store a choice per tool and per agent and is its own spec.
- **Connected on another machine** (§4.3).
- **A catalog search page.** Added when the list passes about 20 cards (§4.1).
- **Changing what any sign-in does.** Every Connect runs the code it runs today.

## 8. Tests

- `connector-entries.test.tsx`: a card's state word and tone for no account, one valid, one expired, one invalid, two accounts with one expired, the AgentMux Cloud session connected and signed out, a text-only connector, a tool server added and one only observed; the order (connected first, then the catalog's); the filter.
- Component tests: Connect opens the right section for each kind in §4.4; a connected card opens its details and Esc closes them; an account's disconnect confirm opens in the details and names its agents; Reconnect from `requestReconnect` opens the card.
- Width tests: `columnsFor` with the step table's widths (280, 339, 340, 565, 566, 841, 842, 1117, 1118, 2400) gives 1, 1, 1, 1, 2, 2, 3, 3, 4, 4 columns with the four-column limit; a card under 340 px wide has the stacked layout and from 340 the row layout whatever the column count; the card order is the same at every width; an open card's details follow the end of its new row after a resize, the caret follows, and the card stays in view.
- Accessibility test: the disclosure and the control are separate buttons, `aria-expanded` follows the state, the label carries the state word.
- Rust: the default tree has four panes in the agreed order and sizes; `window_create` seeds four views; a saved layout is untouched.
- The existing pinned snapshots for the Accounts and MCP sections are replaced, not edited.

## 9. Phases

1. **The list for accounts** (replaces the gallery and Connected grid; MCP and Remotes sections stay), with the responsive cards and the `CardGrid` limits. Debug `task dev` on narko for the operator to try at a thin, a medium and a very wide pane; macOS and Linux look checks at the same three widths.
2. **MCP servers join the list;** the section bar becomes Connectors and Remotes.
3. **Startup:** the fourth pane in the three places (Rust and frontend), sizes tuned on the same debug build; Opaz runs the Rust tests; look checks again for the layout.

Each phase is its own PR with a changeset, through muxreview. The spec and its pictures are committed with phase 1.
