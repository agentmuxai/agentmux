# Deep links: one framework by which any view opens at a named place, in any pane

**Status:** active. Approved by the operator 2026-11-20 with decisions D1 to D12 as recommended, and three additions written in (G4: entities in chrome; G5: rectangles and frame from one instant; R19: the hand pointer). Written against `origin/main` at `c8eef66b52`; §2 re-read against `33e4758fe3` on 2026-11-21. Build 1 (§9) is task #48140.

## 1. What is asked

1. Any view the app has can be opened, in any pane, at a named place inside it (a Settings section or one setting, the Vault pane or one entry, a Connectors card, a Kanban task, an Audit row, a status-bar panel, a file at a line), from MuxAPI, from a link a person clicks, from a toast, from the command palette and from outside the app. Opening a place by an agent is a first-class way for the app to be driven, so it needs guards and its edge cases decided.
2. It is one framework: a destination (a view, a place in it, its parameters) and a target (which pane shows it) are resolved by one resolver, whoever asks.
3. The app's own words that send a person somewhere become links: a toast, notice, empty state or error that names a place in words links to it, underlined, to the exact place. The first one is the voice transcription "not available" toast, whose advice about Settings becomes a link to the exact setting.
4. The stray code that already opens places in its own way is consolidated into the framework, and a check fails on a new one.

## 2. What exists today (read from the code)

Nothing here is new design; it is the inventory the framework replaces or keeps. Paths are under `frontend/app/` unless they start with `crates/`.

### 2.1 What an agent can open

| Tool | Reaches | Limit today |
|---|---|---|
| `Pane` open (`crates/mcp/src/tools/pane_tool.rs`) | `editor`, `media` (the editor following the folder), `files`, each with a path | The view list is three names; the srv side (`crates/srv/src/server/app_api/pane.rs` `build_pane_meta`) also takes `term`, `browser`, `sysinfo`, `help`, `tower`. A raw request that carries `meta` skips the view list altogether (it is how the Widget tool opens a widget pane) |
| `UI` command (`ui_tool.rs`, `keybindings/app-api.ts` `runCommand`) | Only ids of the shortcut table. `app:settings` opens Settings with no section. Nothing in that table opens the Vault, Kanban, Connectors, Memory or Swarm, whose commands exist only in the palette's registry | Three ids are refused (`files:deletePermanently`, `browser:pauseAll`, `browser:resumeAll`) |
| `Browser` open | A web page in a browser pane, `http` and `https` only, with the profile rules | Its own |
| `Agents` open | An agent's pane (focus is forced off for an agent's call) | Its own |
| `Widget` open | A widget's pane (`ext:<package>/<pane>`), the package must be approved | Its own |
| Container agents | `editor`, `media`, `sysinfo`, `help`, `tower`, `files` (a file is snapshotted); not `term`, `browser` or widgets | `crates/srv/src/server/container_pane_snapshot.rs` |

There is no rate limit on opens. An agent's open already takes no focus, and a request for another tab needs the tab to exist.

### 2.2 What the app opens for itself, each its own way

| Opener | Opens | Used by |
|---|---|---|
| `openConnectors(section)`, `openMemory(section, {agentId})` (`store/section-panes.ts`) | Connectors (accounts, mcp, remotes), Memory (global, personal, skills, bundles) | Palette, agent notice card, account strip, failure row, bundle summary, agent menu |
| `openSettingsAt(section, board?)` (`view/settings/open-settings.ts`) and literal `meta` written in two places | One of 14 Settings sections | Status bar, Kanban, fork notice, browser profile menu, voice setup notice, LAN peer row, missing widget |
| `openKeyboardSettings(command?)` (`store/keyboard-focus-request.ts`) | Settings at Keyboard, then a row | Palette, quick tips, the notice after an agent changes shortcuts |
| Settings search (`view/settings/settings-view.tsx` `handleSelectResult`) | A section, then the row `#setting-<id>`, scrolled and highlighted for a moment | The search box; an index entry can instead run a function |
| `openVault(section?, requestId?)`, `openVaultAt(id)` (`view/vault/open-vault.ts`) | The Vault pane, a section or an entry | Status bar summary, the vault request card |
| `openKanbanTask(taskId)` (`util/open-kanban-task.ts`) | Kanban at a task | `agentmux://task/<n>` links, held tasks, reviews |
| `openRemotesInPane` (`view/remotes/open-remotes.ts`) | Connectors at Remotes | Unverified callers |
| Audit row keys (`view/audit/audit-shared.ts`) | Audit at an agent, a sound or a message | Status bar, Messages |
| `openPathLink` (`element/link-destinations.ts`), `openInEditorPaneOnScreen` (`util/editor-open.ts`), `openInEditor` (`view/files/files-open.ts`), `openConfigFileInEditor` (`util/open-config-file.ts`), the `editor:line` meta | The Editor or Files at a path, and a line | Markdown, linkified text, Files menus, Settings footers, Remotes "Edit in ssh config" |
| `openVersionPanel` (`store/app-update.ts`), `goToSummaryRow` (`statusbar/summary-go.ts`) | The status bar's version or Maintenance panel, then one of Vault, Swarm, Kanban, Settings (devices, widgets) | The status bar summary |
| `goToAttention` (`notification/attention-go.ts`), `revealBlock` (`util/reveal-block.ts`), the OS notification click | A pane by block id | Attention list, OS notifications |
| The `Ctrl+Shift+O` chord (`keybindings/defaults.ts`, `host-keys.json`) | Terminal, Files, Swarm, Kanban, Vault, History, System Info by `open:*` commands; Connectors and Memory by `app:connectors`, `app:memory` | The keyboard; Back and Forward through focused panes are separate commands |
| The links recognition layer (`links/entities/openers.ts`, used by `links/linkified.tsx`) | A task (through `openKanbanTask` or `openTaskLink`), a web address, an agent's pane (`goToAgent`) | Kanban card and drawer text. Its openers are thin adapters, to be replaced by this framework's typed wrappers (R11a) |
| Palette entries (`store/command-registry.ts`, `palette-open-commands.ts`, `palette-key-commands.ts`) | One entry per place | The palette |
| `runWidgetCommand` (`block/widget-contributions.tsx`) | A widget's pane | Widget `contributes.commands` |
| `openOrFocusPaneByView` and `openOrFocusPaneIdByView` (`store/block-component-registry.ts`) | The one function nearly all of the above end in: bring a pane of that view forward, or make one | Everything above |

### 2.3 Links, and the strings that name a place

- `element/link-target.ts` classifies an href: `web`, `file`, `task` (`agentmux://task/<n>`, shipped), `blocked` (any other `agentmux:` URL, never sent to the OS), `other`. `element/link-open.ts` opens them (a click opens in the app; Ctrl or Cmd floats it; the middle button opens a browser pane). There is no `agentmux://` registration with the operating system. The only other `agentmux://` URL is the pairing string (`agentmux://pair?…`), which is data for pairing, not a place.
- Words that name a place with nothing to click, today: the voice toast (`frontend/app-init.ts`: "Voice transcription isn't configured — open Settings ▸ Recording to set it up"), the voice-off toast and tooltip (`en.json` `voice.setupNotice.off`, `MicButton.tsx`), the widget bar notice ("Show it again in Settings → Widgets."), the paired-device notice ("Settings > Paired devices can revoke it."), the fork notice ("Settings › Agents shows what would change"), the browser attention card ("added to Settings, Agents on sites"), and several `en.json` lines ("Connectors → Remotes", "Connectors > Accounts", "the Vault", "the Swarm's Supervisor settings"). The same kind of path is written with five separators: `→`, `->`, `>`, `›`, `▸`. The voice tooltip and the voice toast also say different things for the same cause ("unavailable in this build" and "isn't configured").
- Guards that exist and the framework reuses: an agent's pane open takes no focus; a tab that is gone is refused (`TAB_NOT_FOUND`); browser profiles are checked per agent; `operator_only` marks prompts addressed to a person; the keyboard dispatcher refuses an agent's commands while such a prompt is open (`store/keymodel-dispatch.ts`); a singleton view comes back instead of opening a second one (`crates/srv/src/server/app_api/pane_singleton.rs`). Window capture learns whether the Vault is on screen from srv (`/api/v1/vault/on_screen`, read in `crates/mcp/src/tools/ui.rs`) and refuses when it cannot tell (`crates/mcp/src/window_capture.rs`).

## 3. The model

### 3.1 Destination and target

- **R1.** A **destination** is `view`, `place` and `params`. `view` is a registered pane view id (`settings`, `vault`, `kanban`, `editor`, `ext:<package>/<pane>`, …). `place` names somewhere inside it. `params` are typed and listed by the view. A destination with no place opens the view as it is.
- **R2.** A **target** is where it shows: `beside` (next to the caller's own conversation; the default), `pane:<id>`, `new` (a new pane), `tab`, `window`. Only MuxAPI takes a target: a link never names one (§3.2). The caller may not name a target it may not use (§7, G3).
- **R3.** One function resolves a destination and a target to an outcome: `opened`, `reused`, `queued` (the app is still starting, or the open waits for the person: G3, G6) or `refused`, always with the pane it landed in, the destination's label, and for a refusal the reason and what would work. MuxAPI, a click, a toast, the palette and the operating system all call it. There is no per-view special case in it.
- **R4.** The same destination asked for twice reuses the pane that shows it and marks it again; it does not make a second pane. The place is then revealed again (the section selected, the row scrolled to and marked).
- **R5.** "Replace" is narrow. A target pane that shows the same view is reused. A pane the caller opened itself, which shows nothing the person has changed, may be replaced. Never replaced, by anything: an agent pane with a running agent, a terminal with a process, an editor with unsaved changes, a browser mid-form or mid-load of a page the person navigated to, a pane the person has typed in. (Decision D2.)

### 3.2 The address

One grammar, written for both parsers:

```
link      = "agentmux://" view [ "/" place ] [ "?" params ] [ "#" anchor ]
view      = id                              ; settings, vault, kanban, editor, ext.<package>.<pane> …
place     = segment *( "/" segment )
segment   = 1*( unreserved / pct-encoded )  ; an id, never a label
params    = param *( "&" param )
param     = key "=" value
anchor    = id                              ; a place inside the place: one setting, one row
```

**A link never names a target and never opens a connection.** The grammar has no pane, tab, window or connection in it, so a link cannot ask to replace, move or reach anything, or to open an SSH host or a WSL distribution with the person's keys; only MuxAPI takes a target (R2, G3). Such a name written as a parameter is an unknown parameter and is refused (R8).

- **R6.** Ids, never labels, in every segment: a renamed label does not break a link.
- **R7.** One canonical form per place: lower-case view and place, parameters in the registry's order, no empty parameters, no trailing slash. A parser accepts only the canonical form and a short list of declared aliases (`agentmux://task/<n>` is the alias of `agentmux://kanban/task/<n>`, kept because it shipped). Copying a link gives the canonical form.
- **R8.** A link is at most 512 bytes. Percent-encoding is decoded exactly once; a link that decodes to something that decodes again is refused. A parameter appears once. An **unknown** view, place, parameter or a wrongly typed value is refused with a reason, not ignored and not partly opened.
- **R9.** A link written by a newer AgentMux to a view, place or parameter this one does not have says so ("this place needs a newer AgentMux") and is not a silent no-op. The grammar itself is frozen at this version: new places, views and parameters are added by the registry, never by changing how a link is read.

### 3.3 The destinations (the registry's content)

Each row is declared once by its view (§4). "Build" is the PR in §11 that adds it. "Agent" says whether an agent may open it by MuxAPI. "Effect" is what showing it does beyond showing: `none`, or `starts` (starts a process) or `network`. A destination whose effect is not `none` is a **worker**: it keeps its own tool and rules (`Browser`, `Agents`, `Widget`, a terminal) and a link only reveals one that already exists (G1).

| View | Places and parameters | Example | Build | Agent | Sensitivity |
|---|---|---|---|---|---|
| `settings` | section (14 ids); anchor = one setting id; `board` for Kanban | `agentmux://settings/recording#recording.engine` | 1 | yes | public |
| `vault` | section (`api-keys`, `logins`, `kept`, `activity`, …); `entry/<opaque id>`; `request/<id>` | `agentmux://vault/entry/e_4f2a` | 1 | yes (opens the pane; values stay masked) | secret (G5) |
| `connectors` | section (`accounts`, `mcp`, `remotes`); anchor = a card | `agentmux://connectors/accounts` | 1 | yes | public |
| `memory` | section (`global`, `personal`, `skills`, `bundles`); `agent` for personal | `agentmux://memory/personal?agent=a1` | 1 | global yes; personal only for the caller's own, another agent's refused in build 1 (G5) | private (G5) |
| `kanban` | `board/<slug>`; `task/<n>` (the number on the default board); `board/<slug>/task/<n>` elsewhere (`#` is the anchor, so a board's task is a path) | `agentmux://kanban/task/48139` | 1 | yes | public |
| `swarm` | section (`supervisor`, `broadcast`) | `agentmux://swarm/supervisor` | 1 | yes | public |
| `audit` | `agent/<id>`, `sound/<id>`, `message/<id>` | `agentmux://audit/message/m_91` | 1 | yes, its own place only in build 1 (G5) | private (G5) |
| `history`, `atlas`, `reviews`, `drone`, `toolchain`, `tower`, `help`, `shortcuts`, `sysinfo` | the view, and a section where it has one | `agentmux://help` | 1 | yes | public |
| `editor` | `path` (absolute, encoded), `line`, `offset`, `viewer` | `agentmux://editor/C%3A%2Fwork%2Fa.md?line=12` | 1 | yes (G5, G7) | by path |
| `files` | `path`, `select` | `agentmux://files/C%3A%2Fwork` | 1 | yes (G5, G7) | by path |
| `agent` | `id`: reveals the agent's pane when it is open | `agentmux://agent/a1` | 2 | yes (reveal only) | private |
| `statusbar` | panel (`version`, `maintenance`, `host`, `cpu`, …) | `agentmux://statusbar/maintenance` | 2 | yes | public |
| `ext:<package>/<pane>` | what the widget declares | `agentmux://ext.acme.notes/main` | 3 | its package must be approved | by widget |
| `term` | `id`: reveals an open terminal | `agentmux://term/b_77` | 2 | reveal only | private |
| `browser` | `id`: reveals an open page. Opening a page is the `Browser` tool | `agentmux://browser/b_12` | 2 | reveal only | private (a person's profile) |

The first build covers the rows marked 1. Rows marked 2 and 3 are declared in the registry from the first build (so the coverage test knows every view) with `build` set, and open nothing until their build. `media` is an alias of `editor` with `follow_folder`. `identity` is retired. `warden` is Jobs only and is reached through `drone` or its own section as today.

## 4. One registry

- **R10.** Each view declares its destinations once, in one data file in the repo (the registry, `crates/common/deeplinks/destinations.json`; its conformance file sits beside it), and nothing else says what a destination is. A declaration holds: the `id`; the places and their parameters with types and limits; the label a person sees, as an i18n key, formed from the view's name and the place's name by one function (so the separator is one character everywhere); the icon; how to open it and how to reveal the place once open (a section to select, a row to scroll to and mark); `effect`; `sensitivity` and what it forbids (§7, G5); `agent` (may an agent open it, and with which targets); `surfaces` (desktop, web, mobile) and what to offer where it is missing; `build`; `aliases`.
- **R11.** These read the registry and nothing else: MuxAPI's `Pane` open and its refusals; the link handler; the palette's "open" entries; the toast and notice helpers; the link component; the generated reference for the docs; the coverage test; the lint of §5.
- **R11a (callable).** The executor is also a plain function in the app, `openDestination({ view, place, params, target })`, which returns the outcome of R3 (`opened`, `reused`, `queued` or `refused`, with the reason). The link handler, the palette, a toast and a link component all call it; none has a path of its own. Typed helpers wrap it for the things people name most: an agent, a task by number, a file (with a line or an anchor when it has one), a URL (a browser pane, or the person's browser by the rules of G7) and a tab. A layer that recognises names in text (an agent's name, `#48140`, a path, a URL) turns each into a link through the registry's own entity kinds and calls these; it never opens anything itself.
- **R12.** The registry is a data file that both sides generate their typed tables from, with a `--check` that fails when a generated file is stale, as `scripts/gen-rpc-stubs.mjs` does. The parser is written twice (srv in Rust, the app in TypeScript) from the one grammar and the registry, and both run the same conformance file: valid links and the reason for each invalid one, including the encoding and bidirectional cases of G8. Both are fuzzed. (Decision D6.)
- **R13.** A **coverage test** fails when a view registered with the block registry (`block/block-registry.ts`, the pane-tab registry, a widget's panes) has no destination in the data file, when a destination names a view that does not exist, and when a place or parameter has no label.
- **R14.** The registry keeps the `<view>:section` meta contract that saved panes and widget pins already carry (`store/pane-sections.ts`: moved and retired sections keep working). A link's place is written to the same meta, so a pane opened by a link and a pane restored from a saved layout are the same pane.

### 4.1 The palette is fed by it

The command palette (spec `SPEC_COMMAND_PALETTE_TOP_CENTRE_2026_11_17.md` §9) says one registry should say what a command is and the shortcut table which key runs it. This registry is the same idea for places, and the two are one set of declarations, not two:

- A palette command that opens a place **is its destination**: its label, icon, keywords and group come from the destination's declaration, and its `execute` is the one resolver call. The fields the palette reads, all in the declaration from the first build: `label` (an i18n key), `icon`, `command` (the command id, which is also the keybinding id, so the shortcut is looked up by it), `keywords`, `group` (the palette's category), `when` (the same flag names the shortcut table's rows use; absent means always), `rank` (optional, lower first; by default the group's order) and `sensitivity`, so the palette never lists a destination its caller may not open. The command keeps its id (`open:vault`) so shortcuts and `UI` command keep working. A command that does something else (close a pane, change zoom) stays in the command registry as today.
- Shortcuts stay in the shortcut table, by command id. The palette still reads the key with `shortcutFor(id)`.
- **Order of the palette PRs.** The look (PR 1), keys, focus and roles (PR 2) and recently used (PR 3) do not touch how commands are declared and go on. The PR that shows every shortcut and the PR that makes one source of truth wait for this registry's first build, so they are written once, against it.

### 4.2 Where the resolver lives

- The frontend holds the one executor, because the panes, the focus and the marks live there: `openDestination(destination, target, source)`.
- srv holds the guard, because MuxAPI callers and their limits live there: it parses a request with the Rust parser, applies G1 to G9, records the open as the agent, and sends the app a typed `OpenDestination` request that the executor carries out. A person's click skips srv's agent limits (the person is the authority) and goes straight to the executor, which still applies the target rules of R5 and the safety rules of G7 and G8.

## 5. The stray code, and what happens to it

- **R15.** After the build every row of §2.2 is either replaced by a call to the resolver or kept, for the reason in the table. A check (a script and a test, in the same style as `scripts/check-ui-primitives.mjs`) fails on a new call of `openOrFocusPaneByView` or a hand-written `meta` with a `<view>:section` key outside the framework, and on a toast, notice or string that names a place in words without a link (R25).

| Code | Fate |
|---|---|
| `openConnectors`, `openMemory`, `openSettingsAt`, `openKeyboardSettings`, `openVault`, `openVaultAt`, `openKanbanTask`, `openRemotesInPane`, the Audit row keys, the two literal Settings `meta` writes | Replaced: each becomes `open("<link>")` through the resolver; the section manifests (`CONNECTORS_SECTIONS`, `MEMORY_SECTIONS`, `SETTINGS_SECTIONS`) move into the registry |
| Settings search `handleSelectResult` | Kept as the **reveal** for the `settings` view (select the section, scroll to `#setting-<id>`, mark the row). The `opens` functions on index entries become destinations |
| `openPathLink`, `openInEditorPaneOnScreen`, `openInEditor`, `openConfigFileInEditor`, `editor:line` | Replaced by `editor` and `files` destinations; the path rules of G7 are applied in one place, not in each |
| `agentmux://task/<n>` handling in `link-target.ts` and `link-destinations.ts` | Replaced; the old form stays as an alias (R7) |
| `agentmux://pair?…` | Kept, outside the framework: it is data for pairing, not a place; it stays `blocked` as a link |
| `goToSummaryRow`, `goToAnswerPlace`, `openVersionPanel` | Replaced: the status bar summary rows carry a destination |
| `goToAttention`, `revealBlock`, the OS notification click | Kept as the `pane` target by block id; the click carries a destination where there is one |
| Palette open commands | Replaced by the destinations (§4.1) |
| `runWidgetCommand` | Kept for focusing an open widget pane; opening goes through the resolver |
| `UI` command for `app:settings` and the like | Kept: it runs shortcut commands. A `Pane` open with a link is the way to a place; the refusal for a command that has no shortcut id points at it |
| `openOrFocusPaneByView`, `openOrFocusPaneIdByView` | Kept, as the resolver's own implementation of "make or find a pane"; nothing else calls them |
| Widget SDK `panes.open`, `ui.openUrl` | Kept; `panes.open` of a view goes through the resolver and needs the declared permission (G4f) |
| The two different "voice unavailable" texts | One text, in the registry's words, with a link |

## 6. Who emits links

- **R16 (agents).** An agent writes a link in a reply, a message, a task or a document. A MuxAPI refusal that sends a person somewhere carries the link, in the refusal's own field, so the agent can pass it on ("the person sets it in the Vault pane": `agentmux://vault/keys`). An agent opens a place with `Pane` open given a `link`, or a `view`, `place` and `params`, and a `target`; the answer is the outcome of R3.
- **R17 (the app).** Every toast, notice, empty state, error and tooltip that names a place in words contains a place link. The words for a place are produced by the registry's label function (`Settings › Recording`), never typed.
- **R18 (outside).** The operating system opens `agentmux://…` (build 4); AgentMux Web and AgentMux Mobile read the same grammar and, for a place they do not have, say so and say what would work (the registry's `fallback`).

### 6.1 The link component

- **R19.** One component, `PlaceLink`, renders every link the app draws: underlined, in the link colour, a real anchor that is reachable by keyboard and has a visible focus ring, and showing the hand pointer (`cursor: pointer`) wherever a click on it opens something. The component carries the pointer itself, so no caller has to add it; the app-wide sweep of other clickable elements is separate work.
- **R20.** Its text is the text it is given, and its accessible description and tooltip are the registry's real label of where it goes. A link written by an agent whose text differs from the real label shows the real label next to it in muted text, so that text cannot say "docs" and open the Vault. A link the app drew itself shows no extra.
- **R21.** It is sized by whatever holds it: chrome zoom in a panel, a toast or a notice; pane zoom in a pane's content. The component sets no size of its own.
- **R22.** A click on it opens through the resolver as the person's own act. A middle click or Ctrl/Cmd click targets a new pane or a floating pane, as links already do.
- **R23.** In a Browser pane, in a file shown in the Editor or in terminal output, text that looks like a link is not made clickable by this component; §7 G4 says what each of those do.

### 6.2 The strings that name a place

- **R24.** A string that names a place is written with markup the i18n layer understands, `<place to="settings/recording#recording.engine"/>` where the words go, so a translated string keeps its link and the label stays the registry's. The existing `{{ }}` interpolation is not used for this, and a value interpolated into any string is escaped as text and can never carry `<place>` markup: the app's links come only from its own i18n strings (G4).
- **R25.** A check scans i18n strings and the toast, notice and tooltip call sites for the names of the places (Settings, Vault, Connectors, Memory, Kanban, Swarm, Audit, Maintenance, and the ids' labels) and fails on one with no link beside it. Phrases that mean the word in another sense go in an allow list with a reason.
- **R26.** The first conversion is the voice transcription toast: its title and message say what is wrong; "open Settings ▸ Recording" becomes a link to the setting that fixes it (`settings/recording#recording.engine`), and the tooltip on the mic button, which says "unavailable in this build" for the same cause, says the same thing and links to the same place. The same PR converts every string listed in §2.3.

## 7. Guards

A link shows a place and nothing more. These are requirements; each has a test (§12). Guards are numbered G1 to G13; the decisions in §13 are D1 to D12, so an answer such as "D3" means one thing.

- **G1. Showing is all a link does.** Opening a destination never changes a setting, reveals a secret, approves or answers a card, runs a command, sends a message, starts or stops an agent, starts a process or spends money. There are no action destinations. No parameter puts text into a composer, a terminal, a field, or anything that runs or saves on Enter. A parameter may prefill only a search or filter box in which neither Enter nor a click runs anything: a Kanban filter, and the Settings search once every entry of its index is a destination (build 3, §5; until then it takes no prefill, because an index entry can run a function today, §2.2). The command palette is not such a box, since Enter runs its top match, so no link prefills it. A prefilled box never has focus, so no key the person presses lands in it; each prefill is listed in its declaration, capped at 80 characters, shown as plain text and cleared with one key. A worker's effects stay in its own tool; a link to a worker reveals an existing pane and says "not open" if there is none, and offers nothing that would start it.
  - **The one stated exception: an agent the app itself draws.** A person's own click may open, and so start, an agent's pane only where the app draws that agent itself and the visible text is the agent's name: a Kanban chip, a Swarm or My Agents row, the palette, and a name the links recognition layer resolved from text. Such a start sends the agent nothing (no message, no prompt, no turn) and obeys the agent limit like any other start. Everywhere else an agent destination is reveal-only and never starts one: an open by MuxAPI (`Pane` open with a link or a view), an `agentmux://agent/…` address from any source, and a Markdown link in agent-written text whose label is not that agent's name. Tests cover both sides: a click on a chip starts the agent and sends it nothing; the same destination by MuxAPI, by an `agentmux://agent` address, and by a Markdown link with another label, says "not open".
- **G2. No secret in a link.** A link holds ids and places, never a token, a key, a value or a path inside a secret store. A Vault entry in a link is an **opaque id** (`e_<random>`) that the Vault assigns and keeps, not today's id when it spells the entry's label; the Vault resolves it and says "that entry is gone" when it cannot. A link in a public place (a pull request, an issue, a chat) therefore tells a reader nothing about what the entry is. A path in an `editor` or `files` link is a path the person could be shown; G7.
- **G3. What an agent may open, and where.** An agent's open lands beside its own conversation, in its own tab, without taking the person's focus, caret, active tab or window; it does not close, move, resize, magnify or un-magnify anything; and it never happens while it would shift what the person is typing into or about to click. A target outside its own tab is not refused and does not reach the person's tab: the open lands in a tab of its own, without focus, as `Agents` open does for an agent on another machine. The person's own pane and a pane with work in it are never replaced (R5). Naming another window is refused in build 1, with the reason. (Decision D1.) The same destination asked for twice reuses and marks the pane (R4). Limits: ten opens a minute per agent; and a cap that workers cannot multiply past, counted across every agent together: panes agents opened that the person has not yet used (looked at, clicked in or typed in), at most eight in one tab and twenty-four in all. A refusal names the limit and when it resets or what would free it. The person is **typing** when there was a keystroke in a field of the tab the open would change in the last two seconds, when the microphone is recording (dictation), or when an input method is composing. While that holds the open is **queued** until two seconds after it stops, for at most ten seconds, and then opens without taking the caret or, if the layout would shift under the pointer, in a tab of its own.
- **G4. Where a link came from decides what a click needs.**
  - (a) *The app's own text:* a click opens.
  - (b) *An agent's reply the person clicked:* the click is the person's gesture; it opens, and shows the real label (R20) so the text cannot lie about where it goes.
  - (c) *Quoted text from another agent, a file shown in the Editor or a preview, terminal output, a Kanban task or an Audit row:* the text is not made a link by the framework. A place link in a Kanban task's description and in a reply or message the person is reading is clickable by (b); the rest is text, with a "Copy link" in the context menu. (Decision D4.)
  - (d) *Web content in a Browser pane:* a page can never open one by script, redirect, frame or an automatic navigation; `agentmux:` is refused in the page's navigation. A person's click on one shows a **confirm panel in the pane** (a panel, not a window modal) naming the place by its real label, with Open and Cancel; Cancel is the default.
  - (e) *The operating system:* the caller is unknown and a web page can launch it, so it navigates and nothing more: it may not name a target other than the last focused window, may not replace anything, is rate limited to five a minute, waits for the app to finish starting, and says which window it used. In build 4 it refuses every destination shown **by path** (`editor`, `files`): a page could make the browser download a file and then send the person to it in our own preview. A destination whose sensitivity is `public` opens; any other opens the pane only, with no entry or row selected. Every open from the operating system is recorded in the Audit with its source (G9).
  - (f) *A sandboxed widget:* needs the declared permission to open a place and may open only the places its permission names.
  - An agent can never make a link look like the app's own button, card or notice: agent-authored markup is sanitized to the link component's agent form, and no class from the app's chrome is allowed in it.
  - Agent-supplied text shown in the app's own chrome (a task's title, a status line, a pane's or an agent's name, a toast or notice built from an agent's words) is never made a **place** link from its text, and no value interpolated into a string can carry `<place>` markup: it is escaped as text (R24), and a test passes markup through each interpolation. The app's place links come only from its own i18n markup.
  - The one exception is an **entity** that the links recognition layer (`SPEC_LINKS_ON_EVERYTHING_2026_11_20.md`) finds in that text: an agent's name, a task number, a pull request. Such an entity may be a link in chrome, because its target is built from the entity the layer resolved against its own indexes, never from the characters written (that spec's §6.1). The text chooses only which known entity; it can never name a view, a place, a parameter or an address. The layer's click calls this framework's typed wrappers (R11a), so the guards here apply to it unchanged. A test shows that agent text in chrome which spells an `agentmux://` address, or `<place>` markup, stays text, while a recognised task number in the same text is a link.
- **G5. Showing can be reading.** An agent that can open a view and then capture the pane has read it, so opening a destination never gives the caller more than it could already read. What the capture tools do today, read from the code at `c8eef66b52`:
  - `UI` screenshot, query and click act on the caller's own pane or, with `pane`, only on a browser pane the agent opened itself and still owns; the person's take-over ends that. A browser pane's secret fields are blacked out, and a page that could not be checked is not returned (`ui_handlers.rs`, `browser_owner.rs`, `screenshot_redact.rs`).
  - `UI` window captures any window by title or pid, AgentMux's own included, except that no AgentMux window is captured while the Vault is on screen (`window_capture.rs`; the Vault spec of 2026-10-19, §5.3).
  - There is no `Pane` read. `Memory` reads Memory by its own rules.
  - Nothing like the Vault's rule covers the Audit, Memory or a browser pane in the person's own profile: once a link has put one on screen, `UI` window can capture it.

  So build 1 generalises the Vault's rule, without taking window capture away. **In a window capture of an AgentMux window, each pane whose sensitivity is `private` or `secret` is blacked out by its rectangle** (as a browser pane's secret fields are today), and the rest of the window is captured. If the app cannot give the rectangles, the capture is refused, as today's Vault rule refuses it. **The rectangles and the captured frame come from the same instant:** the app reports the layout together with a layout generation, the capture is taken, and the generation is read again; if it changed in between, or the layout is moving (a resize, a split being dragged, a pane animating in or out, a tab switching), or the rectangles cannot be matched to the frame (the window's size or scale differs from the one they were measured for), the capture is refused with that reason, never blacked out from rectangles of another moment. The set that is blacked out is the Vault, the Audit, Memory and a browser pane in the person's own profile. **Agent panes and terminals are not in it**: they are captured today and nothing here changes that, so the look checks and every tester's screenshots go on working. The rule holds in an agent's own test instance too: one rule, so a test instance's Vault is blacked out as well (Decision D12). `UI` screenshot of the caller's own pane is unchanged. The table says who may open each kind and who may capture it:

  | Destination | Sensitivity | Who may open it (MuxAPI) | Who may capture it |
  |---|---|---|---|
  | `settings`, `connectors`, `kanban`, `swarm`, the plain views | `public` | any agent | as today |
  | `vault` | `secret` | any agent opens the pane; values stay masked; an entry by its opaque id (G2) | blacked out in a window capture (today the whole window is refused while the Vault is on screen) |
  | `memory`, global | `private` | any agent (it can read it through `Memory`) | blacked out in a window capture |
  | `memory`, personal | `private` | the caller's own only; another agent's is refused in build 1, because a leader that needs it can read it through `Memory` already | blacked out in a window capture |
  | `audit` | `private` | an agent's own place (`audit/agent/<its id>`); another agent's rows are refused in build 1 | blacked out in a window capture |
  | `agent`, `term` (reveal only) | not in the blacked-out set | an agent reveals a pane that is open; it never starts one (G1) | as today: captured in a window capture |
  | `browser` (reveal only) | `private` when in the person's own profile | an agent reveals a page that is open; it never opens one (the `Browser` tool does) | by `UI` screenshot only the agent that owns the pane, never one in the person's profile; in a window capture a person's-profile pane is blacked out |
  | `editor`, `files` | by path | a path the caller could read (G7); a container agent only through its snapshot rule (`container_pane_snapshot.rs`) | as today; a container agent cannot capture a host path |
  | `ext:<package>/<pane>` | by widget | the package must be approved | as the widget declares |

  - The Vault and Audit panes keep masking as they do; a link opens the pane and the entry, never an unmasked value.
  - The reverse holds: a pane an agent opened for the person stops being capturable by that agent the moment the person acts in it.
- **G6. Misdirection.** A pane or panel that appears must not land under the pointer where an Allow, Approve or Send button was about to be, and an open never moves, covers, scrolls or resizes a card that waits for an answer. A newly shown pane or panel ignores clicks and Enter for a short fixed time. That time is **one constant in one module** of the app's shared frontend code, used by this guard and by the Vault request card's buttons (R3b of `SPEC_VAULT_AGENT_ASKS_TO_BIND_A_LOGIN_TO_A_SITE_2026_11_19.md`, task #48141, which asks for 0.8 seconds): they are one guard and must not drift into two numbers. The number is the person's to set; 0.8 seconds is proposed, and this spec names no number of its own. When the only place an open can go would cover an unanswered card, the open is queued and the pane's tab shows a dot instead.
- **G7. Paths.** `editor` and `files` refuse: a UNC or remote path (on Windows opening one sends the person's sign-in to that server), a device path (`\\.\`, `\\?\`, `CON`, `NUL`), a path with a traversal out of what the caller may read, a path through a link whose target is outside it, and a relative path. A path is normalised once, by one function, before any check, and the check and the open use the same normalised string (no check on one form and open on another). A symbolic link is followed only when its target passes the same checks. **A link never opens a connection:** `Pane` open's `connection` (an SSH host or a WSL distribution) is not a link parameter, because a click would reach a host a link chose with the person's keys, the same class as the UNC path. It is refused as an unknown parameter (R8), and the conformance file has the case.
- **G8. Text safety.** One strict parser per side (R12), fuzzed. Bidirectional and other control characters are stripped from link text and refused in parameters; there is no double decoding; labels are text, never markup, so a label cannot inject any; a link whose text is a different link (an `agentmux:` address as the text of another) shows the real address.
- **G9. Record.** Every open an agent asks for is recorded as that agent (what, where, the outcome, the reason when refused), never as the person, in the Audit the app already keeps. The record is kept for a refused open and for a queued one too, not only for one that opened; an open from the operating system is recorded with its source (G4e); a person's own click is never recorded as an agent's. MuxAPI answers `opened`, `reused`, `queued` or `refused`, with the reason and what would work.
- **G10. No chains.** A destination never opens another; a click is at most one open. A link in a pane that opens opens nothing by itself.
- **G11. No flood.** Clicks on links are limited to five a second; a reply that carries two hundred links is drawn, and none of them opens unless clicked. Opens an agent asks for over the limit are refused, not queued.
- **G12. Check at the time of the open.** Permission and sensitivity are checked when the open happens, not when the link was written or drawn: a link kept in an old conversation opens for whoever clicks it now, with their rights now.
- **G13. Announced.** An open an agent asked for is announced to a screen reader through a polite live region ("Opened Settings › Recording beside this conversation") without moving focus; a person's click moves focus to the opened place's heading.

## 8. Edge cases

Each has a behaviour here and a test (§12) where a test can show it.

| Case | Behaviour |
|---|---|
| The place is gone: a deleted Vault entry, a task moved to another board (its number changed), a section renamed or retired, a closed pane, a setting removed in a newer version | The view opens at its nearest place (the pane, the section), and says why in the pane's own notice ("that entry is gone"). The caller is told `opened` with `place: "missing"`. Never a silent no-op. A moved or retired section follows `pane-sections.ts` |
| The feature is off or not allowed: an add-on that is off (the Browser), a signed-out state, a view a container agent cannot have | `refused` with the reason and what would work (turn it on at a link to the setting that does it) |
| Not loaded yet: a row in a long or virtualised list | Load, scroll, then mark, with a timeout of 5 seconds after which the pane is open at the list's top with a note |
| The app is still starting | `queued`; the open runs when the first window is ready; a queue of at most ten, the oldest refused when full |
| A pane in a hidden tab, a minimised window | An agent's open never raises the window; the tab shows its dot. A person's click may raise |
| Several windows | The window of the caller's pane; for the OS, the last focused window. Never a window the caller does not own; an agent that names another window is refused in build 1 |
| An agent names a target outside its own tab | The open lands in a tab of its own, without focus; nothing of the person's is replaced (G3) |
| A tab the person magnified | The open does not un-magnify it: it opens in the agent's own tab, or is queued with a dot |
| A very thin or very short pane | The pane opens at the size the layout gives it and the place is revealed within it; nothing is resized to fit |
| Zoom | The link component follows R21; the opened pane follows its own zoom |
| OS animation effects off | The mark that shows the person the place is a static outline, held for three seconds, so it is visible with every animation off; the motion policy only removes its fade |
| Another machine: a task on another host's board, an agent on another host | A link carries no host in the first build. A place on another machine is reached by that machine's own link, and the app says "this is on another computer" and which. Build 4 adds a `host` parameter that opens only on a paired, trusted peer; an offline or untrusted one is `refused` with that reason |
| Two opens racing; an open for a pane that is closing | The first wins; the second finds the pane and reuses it, or, for a pane closing, opens a new one |
| A link clicked twice | The second is a re-reveal (R4) |
| A link to the pane the person is already in | The place is revealed in place; no new pane |
| A link in an old conversation after an upgrade | G12; a place that moved follows `pane-sections.ts` |
| A link written by a newer AgentMux | R9 |
| A person has a prompt open that is addressed to them (an approval, a grant) | An agent's open is queued behind it (the dispatcher already refuses an agent's commands while one is open) |
| An agent asks for a destination its container cannot have | `refused`, naming the host-exec reason and the link a person could click |
| A link in a toast after the toast is gone | The link was only the toast's; nothing remains |
| A destination whose pane is a singleton | The singleton is brought forward (as `pane_singleton.rs` does) and the place is revealed in it |

## 9. The build, in small PRs

1. **Registry, parser, resolver and `Pane` open for every view** (R1 to R14, G1 to G3, G5 to G9). The data file and its generated tables, both parsers and the conformance file, `openDestination`, srv's guard and record, `Pane` open with `link` and `target`, the coverage test, the generated reference for the docs. The first build's rows.
2. **The link component, links in replies and the app's toasts** (R16 to R26, G4, G13). The voice toast first, then every string of §2.3. The check of R25.
3. **Consolidation** (R15). Each opener of §2.2 replaced, the lint of R15 on, the palette's open entries fed from the registry (§4.1), then the palette's shortcut and one-source PRs.
4. **The OS handler, other hosts, AgentMux Web and AgentMux Mobile** (R18, G4e). Registration in the installers, the OS handler's refusal of by-path destinations and its record in the Audit, the `host` parameter (a paired peer's id, never an address or a connection), the surface fallbacks.

Each visible PR gets a look check on macOS and Linux, a changeset, tests, and a debug window for the person to try, as for any change to how something looks.

## 10. The docs

A generated page of destinations for the user documentation: a script reads the registry and writes one table per view (the link, the label, the parameters, who may open it, the surfaces it exists on) and the grammar of §3.2, as Markdown that the docs repository takes in. It is generated, never hand-edited, so the page cannot disagree with the app. It ships with PR 1, is taken into the docs when PR 2 merges, and a PR that changes the registry changes the page in the same push (a `--check`). In alpha the page describes only the current release.

## 11. Relation to other work

- The Vault approval card that links to a Vault entry, and the Configure panel of My Agents, are destinations of this registry (`vault/request/<id>` and `agent/<id>/configure`) and are written against it.
- The command palette is fed by it (§4.1).
- The Kanban task link that shipped keeps working (R7).
- The Vault request card's "a new card ignores clicks for a moment" and G6 are one guard with one shared constant (G6).

## 12. Tests, one per requirement

| Requirement | Test |
|---|---|
| R1 to R5 | Component and unit tests of `openDestination`: each outcome; a second open reuses and re-reveals; R5's never-replace list, one case each (an agent pane with a running agent, a terminal with a process, an editor with unsaved changes) |
| R6 to R9 | The conformance file: valid links with their parsed form; each invalid link with its reason (unknown, double-encoded, over 512 bytes, duplicate parameter, wrong type, a place from a newer version, and a `pane`, `tab`, `window`, `target` or `connection` parameter); run by both parsers |
| R10, R11, R13 | The coverage test; a test that every reader of the registry reads the same file (one import) |
| R12 | `--check` on the generated tables; the fuzz run of each parser (a bounded run in CI) |
| R14 | A saved layout and a pane opened by a link produce the same meta |
| R15 | The lint: a fixture with a new ad-hoc opener fails it; the real tree passes |
| R16, G3, G9 | srv tests: an agent's open beside its conversation; a target outside its own tab opens in a tab of its own with no focus; another window refused; the per-minute limit and its reset; the per-tab and total caps hold across two agents; the record names the agent and exists for a refused and a queued open; a person's click is not recorded as an agent's; a queued open while a person types, while the microphone records and while an input method composes |
| R19 to R22 | Component tests: underline, role, focus ring, the hand pointer, label on hover and to a screen reader, the agent form showing the real label, middle click |
| R24 to R26 | The check of R25 on a fixture and on the tree; the voice toast test: the link is there and goes to the setting |
| G1 | A table test over every destination: opening it changes no setting and runs no command (a spy on the stores and the command registry); no parameter reaches a composer or terminal; the agent exception: a person's click on an app-drawn agent starts it and sends it nothing, while MuxAPI, an `agentmux://agent` address and a Markdown link with another label only reveal |
| G2 | The Vault entry id is opaque; a link with a label in the place is refused |
| G4 | Per source: the click, the confirm panel from a page (Cancel default), the OS handler's limits and its refusal of `editor` and `files`, a widget without the permission; agent-supplied text in chrome is not made a place link (an `agentmux://` address or `<place>` markup in it stays text) while a recognised entity in the same text is a link, and markup in an interpolated value is escaped |
| G5 | A container agent's `editor` link to a host path outside its mounts is refused; another agent's personal memory is refused; a window capture blacks out the rectangle of each `private` or `secret` pane and keeps an agent pane and a terminal; with no rectangles the capture is refused; a layout generation that changes between the rectangles and the frame, a layout in motion, or a frame of another size or scale is refused; the same in a test instance |
| G6 | A pane opened where an Allow button waits is queued; a newly shown pane ignores a click for the shared guard time, and the Vault card reads the same constant |
| G7 | Paths: UNC, device, traversal, relative, a link out of the root, a path that normalises differently from its check |
| G8 | Bidirectional and control characters; markup in a label; a link as the text of a link |
| G10 to G13 | One open per click; five clicks a second; a link checked at click time; the live region text |
| Edge cases | One test per row of §8 that can be tested without a second machine |

## 13. Decisions wanted

Each has a recommendation. They are about what the framework allows, so they are best decided before the build.

- **D1. What an agent may open by default.** Beside its own conversation, in its own tab, no focus taken. A target outside its own tab opens in a tab of its own, without focus, as `Agents` open does on another machine; the person's own pane and a pane with work in it are never replaced; naming another window is refused in build 1 (recommended). The alternative is to refuse everything outside its own tab.
- **D2. Replacing a pane.** Only a pane the caller opened itself, or one that shows the same view; never a pane with a running agent, a process, unsaved changes or a form in progress (recommended). The alternative, never replacing anything, makes "any view in any pane" mean only "in a new pane".
- **D3. Limits for an agent.** Ten opens a minute per agent, and, counted across all agents so that workers cannot multiply it, panes agents opened that the person has not yet used: eight per tab and twenty-four in all (recommended). Refusals name the limit.
- **D4. Which text is clickable.** The app's own text and a person's view of an agent's reply, a task's description and a message: clickable, with the real label shown. Terminal output, files in the Editor and previews: not clickable, with "Copy link" (recommended). The alternative is to make every `agentmux://` string clickable everywhere, which widens what a page, a file or a command's output can make a person open.
- **D5. A link from a web page.** Never opens by itself; a click shows a confirm panel naming the place, Cancel the default (recommended). The alternative is to refuse `agentmux:` in a page entirely.
- **D6. Where the parser lives.** One registry file, a parser on each side from one grammar with one conformance file and a fuzz run on each (recommended). The alternative is one parser in srv, with the app asking srv to resolve a link, which cannot draw a link's label while srv is away or starting.
- **D7. Vault entry ids in links.** Opaque ids the Vault assigns (recommended), because a link is copied into public places and today's ids can spell an entry's label. This needs a small change in the Vault, in build 1.
- **D8. Workers in links.** A terminal, a browser page, an agent and a widget are in the registry but a link only reveals one that is open and never starts one (recommended). The alternative, a link that starts an agent or a terminal, breaks G1.
- **D9. The first build's views.** The rows marked 1 in §3.3: Settings, Vault, Connectors, Memory, Kanban, Swarm, Audit, the plain views, Editor and Files. Agents' panes, the status bar's panels, terminals and browsers (reveal only) and widgets come in builds 2 and 3.
- **D10. The mark.** A static outline held for three seconds, with the fade removed under the motion policy (recommended), so the person always sees the place even with animations off.
- **D11. A link's address form.** `agentmux://<view>/<place>?params#anchor` with the Kanban alias kept (recommended).
- **D12. Window capture while a sensitive pane is shown.** Black out the rectangle of each Vault, Audit, Memory and own-profile browser pane in a window capture, and capture the rest; refuse the capture if the app cannot give the rectangles; agent panes and terminals are not in the set; the same rule in an agent's own test instance (recommended). The alternative is to refuse the whole window while any such pane is on screen, which is simpler but takes window capture away from every tester while the person has the Memory or Audit pane open.

## 14. Left alone

- The shortcut table and `UI` command's use of it; a command that is not a place stays in the command registry.
- The pairing string (`agentmux://pair?…`).
- `Browser` open's rules for web pages, the `Agents` open rules and `Widget` open's package check, which the registry lists and calls.
- The OS notification click's use of a block id.
