# Spec: pane modals, not window modals, for sign-in and OAuth; an audit of every window modal (2026-11-02)

**Status:** active. Approved by the operator on 2026-11-03 ("looks good", relayed by AgentY); built under #18805, in the order of §4.
**Owner:** Agent1. **Lane:** AgentX (agent pane, product direction). Coordinated with Agent2-6 (#17125, Vault requests as cards in the agent pane) and Naki-2 (Connectors).

**Progress** (one line per merged PR; `window-modal-sites.json` shrinks with each):
- 2026-11-03, Agent1@narko: rows 1 and 2 (#6343), 6 and 8 (#6354) done.
- 2026-11-04, Agent1@narko: row 7 done: the media permission prompt and "Stop camera and microphone access?" are pane modals over the browser pane that asked (`window/pane-media-*.tsx`, tests in `pane-media-prompts.test.tsx`, baseline down by two; #6449).
- 2026-11-04, Agent1@narko: row 9 done: the Swarm stop and stop-receiving confirms, the Settings fork-slug update and undo confirms, the account delete and MCP server delete confirms are `scope="pane"`.
- 2026-11-04, Agent1@narko: row 4 done: an agent's desktop-control request is an operator-only card in its own pane (`block/desktop-request-card.tsx`, warning frame), from the request's `block_id`; srv's attention event gives the tone and notification. A request naming no pane is still the window prompt. Row 5 (widget install) is not done: `WidgetInstallRequest` carries no pane (`block_id`), though `widget_agent_handlers.rs` knows it (`asking_agent`), so it needs that field from srv first.

The operator, 2026-11-02: "when choosing to sign in via the agent pane, lets have the modal be a pane (not a window) modal. Same when oauth login from connectors. in fact, lets audit all the times we use window modals, they should be very rarely used".

## 1. What there is today (read from origin/main, 7bce3020c)

- **The pane modal already exists.** `<Modal scope="pane">` (`element/modal.tsx`) mounts into a `PaneModalScope` node, makes only that pane `inert`, traps focus, handles Escape through a scope-aware stack (`modal-stack.ts`), clips native browser panes (`usePaneOverlay`) and falls back to the window with a `console.warn` when no scope is provided. Its only users are the Vault pane (six dialogs) and the browser pane's New profile dialog.
- **Three panes host one:** `<ModalLayer scope="pane">` in `agent-block-content.tsx`, `browser-view.tsx` and `vault-view.tsx`. Every other pane has none, so a view there that asks for `scope="pane"` silently becomes a window modal. The Connectors pane is one (`accounts-manager.tsx:175` has a comment saying so and uses `scope="tab"`, which locks the whole tab).
- **The imperative opener is window-only.** `openModal(Component, props)` (`store/modalmodel.ts`, drawn by `modals/modalsrenderer.tsx`) renders every component at the window root, outside every pane. A prompt that a srv event or a hook raises (all of the ones that matter here) has no way to say "in that pane".
- **All tabs stay mounted** (`workspace.tsx:480`: hidden tabs are `visibility`-hidden, not unmounted), so a modal inside a pane survives a tab switch with no extra work.
- **Sign-in in the agent pane is already mostly in the pane.** The login URL box (`AgentAuthPanel`, `AgentBottomPanels.tsx:89`) and the failure row's "Log in" are inline. The one window modal in that flow is the composer chip's "Sign in with another account…": `useAccountBinding.ts:50` opens `AnotherAccountLogin` (`scope="window"`), which is `ClaudeLoginPanel`. The agent signing in with a saved sign-in (#6182) asks the person through `UsePrompt` (`vault-use-prompts.tsx:131`, `scope="window"`, operator-only).
- **Connector OAuth is a pane problem, not a modal one, apart from one dialog.** `OAuthConnectPanel` and the Connect chooser (`AccountsGallery.tsx:105`, `AgentMuxConnectPanel.tsx:234`) are inline `role="dialog"` blocks inside the Connectors pane. The dialog that locks too much is `ClaudeLoginPanel` there (`accounts-manager.tsx:177`, `scope="tab"`).

## 2. The rule

A dialog about **one pane or one agent** is a pane modal (or, for a request an agent is waiting on, a card in that agent's pane, §3): it covers only that pane, every other pane, tab and window control stays usable, and it lives and dies with the pane. A **tab-wide** question may be a tab modal. A **window modal is kept only for what is about the whole app or window and cannot be answered from a pane**: quitting or closing a window, switching or deleting a workspace, an app upgrade, the command palette. A native OS dialog (a file picker) is not a modal in this sense. Nothing may use `window.confirm`, `alert` or `prompt`: they freeze the whole renderer. Reviewer's test: "if I answered this in another pane, would anything else change?" If not, it is not a window modal.

## 3. The primitive: one, shared with #17125

Agent2-6's spec (`SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md` §4.1) and this one use **one** shape; the names are theirs:

`paneRequests.push(blockId, { source, Component, props, presentation?, deadlineMs? }) -> { close }`, in `frontend/app/view/agent/pane-requests/`, keyed per block. Whichever build is approved first creates the folder; the other consumes it.

What this spec adds:

1. **`presentation: "card" | "modal"`** (default `card`). A card is drawn by the agent pane's dock (`PaneRequestDock` in `AgentBottomPanels`, Agent2-6's). A `modal` entry is a multi-step form the person must finish in place (the Claude sign-in with its URL and code, an OAuth connect) and is drawn through the pane's `<ModalLayer scope="pane">`, so it works for the browser and Vault panes too.
2. **A pane-modal host in the block frame for every pane type**, not only three. `blockframe.tsx` renders `<ModalLayer scope="pane">` around the view once, so a view anywhere (Connectors, Swarm, Settings, Editor, Files) can use `<Modal scope="pane">` or `<ConfirmModal scope="pane">` with no wiring; the three hand-placed layers are removed. A missing provider stays a dev-time error (`data-scope-fallback`).
3. **`openPaneModal(blockId, Component, props)`** for code with no component tree (srv events, hooks): sugar over `paneRequests.push(..., { presentation: "modal" })`.

Behaviour when the pane is not as simple as "open":

| Situation | What happens |
|---|---|
| Too small | The panel is `size="fit"` and fills the pane (the existing compact variant, `@container modal-mount (max-width: 400px)`); it scrolls inside the pane and never spills over a neighbour. |
| Pane in a hidden tab | Nothing different: the tab is mounted. The tab shows the waiting badge (§ below), and the modal is there on return. |
| Minimized window, or pane off screen | No window fallback. The entry stays keyed to its block; for a request an agent waits on, `startWaitingForYou(blockId, source)` gives the tone, tab badge, OS notification (its click raises the window and the pane, as in #18210) and Swarm chip. |
| Parked agent (no pane) | The entry waits keyed to the agent; opening the agent from My agents mounts the pane and shows it (Agent2-6 §6). |
| Pane closed while open | The modal unmounts and its `onClose` runs the safe answer: a request is denied, a sign-in is cancelled (`ClaudeLoginPanel` already cancels the CLI login on cleanup). |
| Focus and Escape | Focus is trapped inside the panel, restored to the opener on close; Escape closes only the topmost modal *of that pane* (`modal-stack.ts` already treats overlapping and disjoint locks that way). Other panes keep their keyboard focus and shortcuts. |
| Operator-only answers | `operatorOnly` (agents' UI tool cannot read or press it) is kept as is. |

## 4. The audit: every window modal, and what to do with it

"Window" below means the dialog covers the whole window today. Sites are `file:line` of the open call (`frontend/app/`). **Build order** is the order of the list.

### Move to the pane (build in this order)

| # | Dialog | Site | About | To |
|---|---|---|---|---|
| 1 | Sign in with another account (agent chip) | `view/agent/failure/useAccountBinding.ts:50`, `AnotherAccountLogin.tsx:14` | one agent | `openPaneModal(blockId)` (`modal`); the hook takes the pane's `blockId` |
| 2 | Claude sign-in in Connectors | `view/accounts/accounts-manager.tsx:177` (`scope="tab"`) | the Connectors pane | `scope="pane"` through the block-frame host |
| 3 | Agent signs in with / uses a saved Vault entry; Vault requests | `view/vault/vault-use-prompts.tsx:224`, `vault-request-prompts.tsx:67` | one agent | cards in the agent's pane (#17125, Agent2-6) |
| 4 | An agent asks to control a native app | `block/desktop-requests.tsx:106` (shown in every window) | one agent | card in that agent's pane (operator-only) |
| 5 | An agent asks to install a widget | `block/widget-install-requests.tsx:92` | one agent | card in that agent's pane |
| 6 | Shut down this agent | `view/agent/shutdown/shut-down-agent.ts:86` (has `blockId`) | one agent | `openPaneModal(blockId)` |
| 7 | Stop capturing / media permission for a page | `window/pane-media-capture-indicator.tsx:148`, `pane-media-permission-prompt.tsx:128` (both `scope="window"` today) | one browser pane | pane modal in that browser pane (the capture state already has `blockId`) |
| 8 | Replace this pane? | `store/keymodel.ts:146`, `modals/replace-pane-confirm.tsx` | the focused pane | `openPaneModal(focusedBlockId)` |
| 9 | Delete / stop / migrate inside a view | `view/swarm/swarm-fleet-toolbar.tsx:149`, `swarm-row-registration.tsx:131`, `view/settings/sections/fork-slug-migration.tsx:411,424` (window); `view/identity/identity-accounts-tab.tsx:447`, `view/mcp/mcp-manager.tsx:360` (`tab`) | the pane that shows it | `scope="pane"` (one word each, after the host exists) |
| 10 | Close this pane / leave with busy agents | `tab/tabcontent.tsx:283,300` (window) | the panes of this tab | `scope="tab"` |
| 11 | Delete a file, discard changes, delete a bundle or memory entry | `view/editor/editor-view.tsx:1281` (`ConfirmDialog`), `view/editor/editor-model.ts:1246`, `view/bundle/bundle-manager.tsx:117`, `view/global-bundle/global-bundle-manager.tsx:91`, `GlobalMemoryFullView.tsx:171`, `view/native-memory/native-memory-manager.tsx:180,687`, `view/memory-editor/editor-keys.ts:41` (all `window.confirm`) | the pane | `<ConfirmModal scope="pane">`; delete `components/confirm-dialog.tsx` |
| 12 | File conflict, attachment viewer | `view/files/files-view.tsx:1488` (fixed overlay in a `Portal`), `view/agent/attachments/AttachmentLightbox.tsx:93` (fixed, whole window) | the pane | draw into the pane (`scope="pane"`) |

### Keep as window modals

| Dialog | Site | Why |
|---|---|---|
| Close window, remove window | `window/window-close.ts:132,162`, `window-close-confirm.tsx` | the whole window |
| Switch, create, rename, pick, delete, import workspace | `window/workspaces.ts:72,123,137,147,175,180,213`, `workspace-modals.tsx` | the whole window |
| Close a tab | `tab/tab-close-confirm-modal.tsx:18` | asked from the tab bar, which a pane or tab lock would leave out of the lock; it ends agents in every pane of the tab |
| App upgrade request | `block/upgrade-requests.tsx:96` | restarts every agent |
| Command palette | `store/command-registry.ts:568`, `store/keymodel.ts:63`, `window/hamburger-menu.tsx:212` | an app-wide launcher |
| Config error, reset the token counter | `statusbar/ConfigStatus.tsx:14`, `statusbar/TokenBreakdownPopover.tsx:286` | the status bar is the window's |
| srv's user-input request | `store/global.ts:324` (`UserInputModal`) | it carries no pane or agent; if srv adds a `block_id` it moves (a one-line follow-up) |
| Native file pickers | `crates/cef/src/commands/platform.rs`, `vault_files.rs` (`rfd`) | the OS's own dialog, not a webview modal |

That leaves **seven** kinds of window modal, all app-wide.

## 5. How the build is checked

- A guard test (like the motion guard) lists the window-modal sites that remain (the keep table) and fails on a new `scope="window"`, a default-scope `ConfirmModal`, an `openModal(` call outside the allowlist, or a `window.confirm`/`alert`/`prompt`. New dialogs about a pane then have to say why they are an exception.
- One test per moved dialog: it opens inside a pane, the sibling pane stays interactive, Escape closes only it, and closing the pane runs the safe answer.
- Screenshots (dark and light) of the sign-in modal in an agent pane and in Connectors, with a second pane live beside it.

## 6. Decisions

| # | Decision | Recommendation |
|---|---|---|
| D1 | One primitive with #17125 (`paneRequests.push`, `presentation` card or modal), no second one. | Yes (agreed with Agent2-6). |
| D2 | A pane that is not on screen gets **no window fallback**; the waiting registry surfaces it. | Yes (Agent2-6's rule, and the operator's). |
| D3 | The block frame hosts the pane modal for every pane type. | Yes; it removes three hand-placed layers and the silent window fallback. |
| D4 | Widget install (row 5) and desktop control (row 4) become pane cards, not window prompts. | **Agreed by AgentX (2026-11-02)**, with three conditions for the build: (1) both stay `operatorOnly` and are drawn from AgentMux's own text, so the agent's words never shape a card's title or buttons and an agent can't read or press them; (2) both register with `startWaitingForYou`, so the tone, tab badge, Swarm chip and the #18210 toast find them when the pane is off screen, minimized or parked; (3) desktop control keeps a clearly stronger look than a routine card, because it controls a native app. AgentX also accepts rows 8 and 12 and D1-D6 and the keep table; the operator approves the spec as a whole through AgentY. |
| D5 | Closing a tab (and a window) stays a window modal. | Yes. |
| D6 | Add the guard test (§5). | Yes: without it the count grows back. |

## 7. Out of scope

The Vault request cards themselves (Agent2-6, #17125); the toast for a waiting agent (#18210); restyling the dialogs; the Google verification flow.

Agent: Agent1@narko
