# SPEC: start a new agent inline, below the harness card, and one CLI-detection truth for picker and Toolchain

Status: spec only, nothing implemented. Written 2026-10-18 (AgentX@narko) at the operator's request:
- "the agent pane should essentially reflect what the toolchain knows .. are those paths DRY? if grok isn't in the path or
  appears not installed, simply put the green, otherwise not."
- "the UX of starting an agent: pivot away from modals, to fields that appear by offsetting neighbouring cards. When adding
  an agent, the new information is entered into a field below the harness button. If we don't have a pattern, design one
  and add it to our design standards."

Companion: `SPEC_AGENT_CLI_ISOLATION_NOTE_2026_10_18.md` (the "AgentMux manages its own CLIs" note). This spec replaces its
§4.4 Step A/B detection and ribbon design; its note, installer table and docs changes stand.

## 1. Part A: the picker shows what the Toolchain knows

### 1.1 Today: not DRY (read 2026-10-18 on `main` 64543db51)

| | Picker card (`AgentPicker.tsx` `checkInstalled`) | Toolchain row (`toolchain-view.tsx` `probe`) |
|---|---|---|
| RPC | `install.check` via `install-check-cache.ts:36` (own 30 s cache) | `resolvecli` with `npm_package: ""`, `pinned_version: ""` for every row (`:199-201`) |
| Backend | `install_handlers.rs:490-522`: managed dir only, never PATH | `cli_handlers.rs:33-125`: managed dir, then PATH **whenever the caller's `npm_package` is empty** (`:92-93`) |
| The six PATH-only providers | never checked: early return when `npmPackage` is empty (`AgentPicker.tsx:376-378`), so never a ribbon | detected, including the vendor folders (`resolve_provider_cli_on_path`, `cli_handlers.rs:848-878`; Kimi has no vendor folder, PATH only) |
| An npm provider whose managed copy is missing | "Click to install" | **"found", pointing at a copy the user installed globally**, because the Toolchain sends an empty `npm_package` and the backend trusts it. Agents never run that copy (`cli_handlers.rs:29`). A real bug. |

A third and fourth copy of the lookup live in `server/app_api/agent_open.rs:487-500` and `ambient/tasks.rs:55-58`.
`install.check` and `resolvecli` also find the npm shim differently (`.cmd`/`.exe`/none vs `.cmd` only).

### 1.2 Change

- **Backend, one lookup.** `backend::cli_install::locate_provider_cli(provider_id) -> Option<Located { path, source,
  version }>`: the managed dir (shared, then legacy); then, **only if the provider registry's `npm_package` is empty**,
  `resolve_provider_cli_on_path`. npm vs PATH comes from the registry, never from the caller. `install.check`, `resolvecli`
  (its lookup half), `agent.open` and the ambient task all call it. `install.check` gains `path` and `source`. Installing
  stays a separate, explicit action (`resolvecli` with a non-empty `npm_package` installs when missing today, `:127+`:
  keep that out of any probe).
- **Frontend, one store.** Provider rows in the Toolchain and the picker cards both read one cache, keyed by provider:
  `install-check-cache.ts` (`checkProviderInstalled`, shared in-flight requests, 30 s reuse, `invalidateInstallCheck`),
  which already served every picker. The Toolchain's provider rows stop calling `resolvecli` and read it too; a forced
  re-check invalidates it first. (As built: simpler than moving both onto `toolchain-capabilities.ts`, which keeps
  serving the core tools.) Version drift keeps the shared `resolveDrift`.
- **Until Part B**, clicking a card whose CLI AgentMux doesn't install never opens the npm install modal: it goes on to
  create the agent as before, and the launch names the vendor's installer (AMX-CLI-004).
- **The ribbon, per the operator.** A card shows the green "Click to install" ribbon (`AgentCard.tsx:188`,
  `_picker.scss:321-338`, `--success-color`) exactly when the store says the provider's CLI is **not found**, for every
  provider, the six included. Found: no ribbon. Still checking: no ribbon. So Grok missing from PATH and from `~/.grok/bin`
  shows the ribbon; installed, it doesn't. The Toolchain row and the card can no longer disagree.
- **Labels in the Toolchain:** keep `managed` for `source === "local_install"`; add `your install` for `system_path` (the six).
  A missing CLI of the six shows its vendor installer's command with a copy button (from the provider catalog).
- Clicking a card with the ribbon opens its inline panel at the install step (§2.3).

## 2. Part B: start a new agent inline, not in modals

### 2.1 Today's flow (up to three modals)

From `handleTemplateSelect` (`AgentPicker.tsx:871-952`), in order, each through the pane's `ModalLayer`
(`modal-layer.ts`, `ModalLayer.tsx`, `modal-dispatch.tsx`):

1. **Missing tools**, `AgentPrereqModal.tsx` (`agent-prereqs`): git/node/npm/python, each with a link or an in-app install
   (`SystemToolInstallInline`), refresh, proceed. Skipped when nothing is missing.
2. **Install the CLI**, `AgentInstallModal.tsx` (`install-agent`): the npm install streaming in `<InstallDialog>`,
   Cancel / Close / Continue to Launch. Skipped when installed or not npm-installable.
3. **Create the agent**, `AgentCreateFromTemplateModal.tsx` (`create-from-template`), always: **Name** (required),
   **Runtime** (host / safe sandbox, the latter gated on CLI support and Docker), **Model** (when the harness has models
   and a `--model` flag), **Custom endpoint** (providers with a configurable base URL), **Bundles**. The account is
   auto-picked (there is `accountId` state in the component; confirm whether a picker shows with several accounts).
   Submit clones the template (`agentdefcreatefromtemplate`) and launches; Escape is blocked while it runs.

Not in this flow: auth, which is an in-pane notice, not a modal (`flows/launch-flow.ts:15-21`), out of scope here; and
`AgentLaunchModal` (working folder, effort, continue a session; `buildLaunchRequest` and `openLaunchModal` in
`AgentPicker.tsx`), which no New Agent card reaches. Per the operator, existing agents have no launch modal either. Part B
checks whether anything still opens it; if nothing does, it is deleted with the other three.

### 2.2 The pattern already exists

`CardGrid` + `Card` (`frontend/app/element/ui/CardGrid.tsx`, `card.scss`) does exactly "fields appear below the card,
pushing the neighbours down": with `selectedKey` and `renderDetail`, it inserts a full-width detail row (`grid-column:
1 / -1`) after the last card of the open card's row, with a caret pointing at the card; it measures its own column count
(`ResizeObserver`), scrolls the panel into view without losing the card, closes on Escape and returns focus to the card,
and keeps the panel through list refreshes so a half-typed form survives. Accounts, MCP servers (including its "New
server" form) and Remotes use it. The rule was written only in a feature spec
(`SPEC_CONNECTORS_CARDS_2026_10_13.md` §3.4); this spec promotes it to the design standards as pattern **P1**
(`SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md` §5.7, added alongside this spec).

The picker grid is already a CSS grid of the same kind (`_picker.scss:170-180`, `repeat(auto-fill, minmax(240px, 1fr))`,
`.agent-picker` the scroll container), so it can move onto `CardGrid` with `minCardWidth={240}`.

### 2.3 The new flow

The "New Agent" grid becomes a `CardGrid`. Clicking a harness card opens **its panel below its row**; clicking it again
closes it; clicking another card moves the panel. Inside, one panel walks through what used to be three modals, showing
only the steps that apply:

1. **Needs** (only if a system tool is missing): the missing tools as rows with the existing `SystemToolInstallInline`
   install and a "Check again". The panel moves on by itself once nothing is missing.
2. **Install** (only if the CLI is not found):
   - npm-managed harness: the install streams inline (the body of `<InstallDialog>`: plain steps, raw output under
     Details), with Cancel. On success the panel moves on.
   - PATH-only harness (the six): the vendor's installer, from the catalog (link plus the command with a copy button,
     `SPEC_AGENT_CLI_ISOLATION_NOTE_2026_10_18.md` §4.3), a line saying it installs outside AgentMux's isolation, and
     "Check again", which re-probes through the shared store (and finds the vendor folder even though srv's PATH is stale).
     A one-click run of the vendor script is not part of this spec (its own security design, isolation note §4.4 Step B).
3. **Create**: the fields of today's create modal, unchanged in meaning: Name (focused on arrival), Runtime, Model,
   Custom endpoint, Bundles; footer `[Cancel: neutral] [Create and start: accent]` (line-style tones, §5.3 of the
   standards spec). Errors show on the field (`Field error`), not in a toast. While creating, the button is busy and the
   panel can't be closed (today's Escape block); when the agent starts, the pane changes to the agent as today.

A small step line at the top of the panel ("Install · Create", the steps that apply) shows where the user is.

**Behaviour decisions** (confirmed by the operator 2026-10-18):
- **One panel at a time** (CardGrid). A half-filled form is kept per harness in memory, so moving to another card and back
  does not lose it; it is dropped when the picker closes.
- **An install keeps running if the panel closes.** Installs are shared and already cancellable; the card shows
  "Installing…" in place of the ribbon, and reopening the panel shows the stream. Cancel is explicit.
- **When an install ends with its panel closed, a toast says so** through the consolidated toast system
  (`pushNotification`, `store/flash-notifications.ts`; shown in every build since #5030,
  `SPEC_RECOVERY_NOTICES_AND_TOASTS_2026_10_16.md` §4.5): "<Harness> is installed" (`type: "info"`, expires) or
  "<Harness> didn't install" with the reason (`type: "error"`), so the user knows to come back. With the panel open, the
  panel itself says it and no toast is shown.
- **Keyboard:** Enter or Space on a card opens it (`Card` already does); focus goes to the first actionable control of the
  current step; Escape closes (not while creating), focus returns to the card; Enter in Name submits when the form is valid.
- **Motion:** none today in `CardGrid`. Add a short height/opacity reveal to `CardGrid` itself (motion tokens,
  `SPEC_DESIGN_SYSTEM_2026_04_23.md` §5.5), off under `prefers-reduced-motion`, so every user of the pattern gets it.
- **Narrow panes:** one column; the panel sits right under the card.

### 2.4 What changes in code

- `AgentPicker.tsx`: the `agent-picker-list` `<For>` becomes `CardGrid` with `selectedKey`/`renderDetail`;
  `handleTemplateSelect` stops opening modals and sets the selected card; the `modalLayer.open/replace` chains for
  `agent-prereqs`, `install-agent` and `create-from-template` go.
- New `AgentCreatePanel.tsx` (the panel and its steps). The bodies move out of the three modals into components the panel
  composes: the prereq rows, the install stream (from `InstallDialog`), and the create form (from
  `AgentCreateFromTemplateModal`). The modal kinds are removed once nothing opens them; check other openers first
  (`modal-dispatch.tsx:205-262`).
- `CardGrid`: the reveal animation, and a way for the panel to block close while busy (an `onClose` that can refuse).
- Tests: `AgentPicker.test.tsx` (the grid opens a panel, steps show and skip correctly, the ribbon follows the store),
  `CardGrid` (close refused while busy), and the moved form keeps its existing assertions.

## 3. Order

1. **Part A** (one detection truth, the ribbon for all providers, the Toolchain labels): backend + frontend, one PR. It
   fixes a wrong Toolchain answer today, so it goes first.
2. **Part B** (inline create) plus the standards entry: one PR, after A, since the install step reads A's store.
3. The isolation note (companion spec) can ride with either.

## 4. Open questions

1. The account: does today's create modal show an account picker when there are several accounts? If so it moves into the
   Create step as is; if not, keep auto-pick.

Settled 2026-10-18 by the operator: keep a per-harness draft; keep an install running after the panel closes and toast
its end. Existing agents have no launch modal (only creating an agent leads to one), so nothing else follows.

## 5. Not in scope

A one-click run of vendor install scripts; the auth notice; the agent picker's other tiers.
