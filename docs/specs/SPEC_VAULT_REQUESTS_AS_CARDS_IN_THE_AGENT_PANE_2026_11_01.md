# SPEC: an agent's Vault and secrets requests ask inside its own pane, as a card with a 30-second veto, not as a window-wide modal

**Status:** active. Approved by the operator on 2026-11-03 ("looks good", with Agent1's pane-modal spec, relayed by AgentY). Spec task #17125; the build is #17126, first PR #6331 (the shared `paneRequests` primitive).
**Author:** Agent2-6 (Vault). **The agent pane's part (§4, §6, §7, §10) was reviewed by AgentX on 2026-11-01**: "ok, with two small additions" (the stack's height cap and the card's own modules, both in §4.1). Agent1 owns the UI kit pieces it reuses.
**Verified on:** `main` at `d2e584f0f` (2026-11-01).
**Extends:** `SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md` §6.4 (the use prompt) and `SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md` §4.4 and §7 (rotation, the agent's sign-in).

## 1. Summary

The operator, 2026-11-01:
- "those requests from agents (vault and secrets) currently commendeere the entire window. instead they should go into the agent pane, and use a modal similar to the brwoser approve or even the qestuon command (not modal) we want to steer away from modals in general"
- "yes, and include the same sound tone"
- "also, give them a timeout, 30 seconds and they can proceed on their own". Asked what silence should mean, they chose "Approved, except IDs and cards".

So:
1. **A card in the asking agent's pane**, docked above its composer with the question card and the permission panel, replaces the window-wide modal. It names the agent, the entry (its label, never a value) and what the value is for, and the rest of the window stays usable.
2. **A 30-second veto.** For API keys, sign-ins, notes and rotations, the card counts down with Deny; if nobody answers, the request goes ahead, recorded as approved by timeout. **ID and card entries never go ahead on silence**: they wait for an explicit Approve and expire as denied.
3. **The same attention tone** as the question card and the browser's Approve banner, through the same "waiting for you" registry, with the same settings, and once, however many requests wait.
4. **Out of sight is not out of mind**: the tab badge, the Swarm chip, the OS notification and, for an agent with no pane, its My agents row and the status bar say a request waits, and its countdown, and open the card. Never a modal.
5. **Only the real control approves.** The card is app chrome built from srv's own record of the request, marked operator-only, answered through the host's route; nothing an agent writes can draw or press it.

## 2. What exists today (from the code)

| Area | Today | Where |
|---|---|---|
| Use prompt | A window-wide `Modal` (`operatorOnly`): the agent, the entry's label and kind, the site and field or the command, why. Deny, Allow once, Always on <site> | `frontend/app/view/vault/vault-use-prompts.tsx:37-164` |
| What asks it | `Browser` `fill` with `vault`, `Terminal` `run` with `secrets`, `capture` with `replaces`, `rotate` (set and verify); B3's `Browser` `signin` (in review) | `server/browser_vault_fill.rs`, `server/shell_secrets.rs:73-82,190-221`, `server/vault_rotate.rs:110-127` |
| Its queue | One for all agents, 16 at most, the oldest denied when full | `backend/vault/uses.rs:100-136` |
| Its wait | The agent's call blocks: 120 s for a command, 10 min for a browser fill | `shell_secrets.rs:29`, `browser_vault_fill.rs:31` |
| Its answer | The window relays it to srv's host-only route, `POST /api/v1/host/vault` `answer_use` | `server/vault_handlers.rs:179` |
| Request prompt | "Fill it in / Not now" `ConfirmModal`, operator-only; the form opens in the Vault pane | `frontend/app/view/vault/vault-request-prompts.tsx:20-46` |
| Its queue and wait | 16 pending, kept an hour; the agent gets `pending` with an id at once and asks again | `backend/vault/requests.rs:84-128,431-459` |
| Which pane asked | Known when the request is made (the agent's verified identity, the shell's `agent_block_id`, the browser's target pane) but **not stored** on `UseRequest` or `VaultRequest` | `uses.rs:18-51`, `requests.rs:22-34`, `vault_handlers.rs:394-403` |
| Attention while waiting | srv publishes a `userattention` event with the pane's id | `backend/user_attention.rs:25-37`; `shell_secrets.rs:139`, `vault_handlers.rs:444`, `browser_vault_fill.rs:217` |
| The question card | `AgentQuestionPanel`, docked in `AgentBottomPanels` above the composer, built from a structured `ToolNode`, queued (head shown, count in the header), 30 s default timeout that applies the recommended answers, deferred by activity, never takes focus | `frontend/app/view/agent/components/AgentQuestionPanel.tsx`, `AgentBottomPanels.tsx:122-142` |
| The permission panel | `AgentDecisionPanel`: Allow or Deny a pending tool call, same dock | `components/AgentDecisionPanel.tsx`, `hooks/useAgentDecisions.ts:110` |
| Waiting for you | One registry of everything a pane waits on the person for. The first source starts the pane's `waiting-for-input`: the looping waiting tone, the OS notification, the taskbar badge, the Swarm's "Waiting for you"; the last ends it. Requests with no pane are kept under an `app:` key | `frontend/app/notification/waiting-for-you.ts:1-90` |
| The tone's settings | `notify:sound:agent.waiting.for.input` and the waiting volume in Settings, Sounds | `frontend/app/view/settings/sections/sounds-section.tsx:100-140`, `notification/sound/sound-service.ts:266-277` |
| The Swarm chip | `working`, `question` or `idle`; a question shows its countdown from a shared timer published in block meta | `frontend/app/view/swarm/swarm-status-chip.tsx`, `frontend/app/store/question-timer.ts` |
| OS notification | `waiting-for-input` becomes `input_waiting`; a click focuses the window, tab and pane | `frontend/app/notification/os/os-notify-bridge.ts:62-126` |
| Anti-forgery | `data-operator-only` (an agent's `UI` query, click and keys refuse it and anything holding it), `ctx.operator_only`, host-token routes, and a markdown sanitizer with no `<button>` | `crates/cef/src/browser_api/scripts/query.js:93-99`, `frontend/app/element/markdown.tsx:144-173` |
| Parked agents | An agent with no pane has a My agents row with a dashed border and a "no pane" tag, and "Open it" | `frontend/app/view/agent/components/MyAgentsList.tsx:38-41` (#6035) |

## 3. Goals and non-goals

Goals: every Vault or secrets request an agent makes is answered in that agent's pane; the window is never blocked by one; the person hears and sees that it waits wherever they are; silence approves only what the operator said; nothing an agent produces can approve.

Non-goals:
- What a request may do once approved: unchanged (the agent never holds a value).
- Requests the person starts (export, import, reveal, copy): they are the person's own, and stay where they are.
- Joining a machine to a shared Vault (`SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md` §8.2): a person's approval with a matching code; no timeout.

## 4. The card (agent pane: for AgentX's review)

### 4.1 Where

The card is a third kind of bottom panel in `AgentBottomPanels`, beside `AgentQuestionPanel` and `AgentDecisionPanel`: docked above the composer, outside the scrolling transcript, the same width and chrome. When the pane also has a question or a permission waiting, the panels stack in arrival order and each keeps its own controls. It is built with the kit's `Button` (and `Menu` for "Always…" choices), and marked `data-operator-only` like the Vault's prompts today.

It reads srv's list of pending Vault requests (the `VaultEvent` it already publishes) filtered to its own pane, so the card is drawn from srv's record of the request, never from anything in the transcript.

**Height.** The card, the question panel and the permission panel share the space above the composer, so the stack is capped at 40% of the pane's height: the head request is shown in full and the rest collapse to §7's "N more" line. Three waiting items never push the transcript off screen.

**One primitive, shared with #18804.** Agent1's spec (#18804: the agent pane's sign-in and connector OAuth asking inside the pane) needs the same thing, so both use one module, `frontend/app/view/agent/pane-requests/`, built by whichever build is approved first:
- `paneRequests.push(blockId, { source, Component, props, deadlineMs?, presentation? })` returns `{ close }`, keyed per pane. `presentation` is `"card"` (the default, and every Vault request) or `"modal"`, for #18804's multi-step sign-in forms, which render in the pane's `<ModalLayer scope="pane">` and cover only that pane;
- a `PaneRequestDock` mounted once in `AgentBottomPanels`: it stacks entries in arrival order with the height cap below, is `data-operator-only`, and never takes the focus;
- each entry starts and ends its pane's "waiting for you" source (`startWaitingForYou` / `endWaitingForYou`), so §6's surfaces come with it;
- **no window fallback**: an entry for a pane that isn't mounted stays keyed to that pane, shows when it mounts, and §6's surfaces say it waits meanwhile;
- a pane with no dock (browser, Vault) renders the same entries in its own `<ModalLayer scope="pane">`.

**Modules.** The card lives in its own module, `frontend/app/view/agent/components/vault-request-card/`; `AgentBottomPanels` only mounts it. The My agents row's request line (§6) is its own small component too. Both `AgentBottomPanels` and `MyAgentsList` are at the file-size gate. Any piece added to the UI kit is agreed with Agent1, so the kit stays one set.

### 4.2 What it shows

| Request | The card says | Controls |
|---|---|---|
| A use (fill, command, capture, sign-in) | "**Builder** wants to use **GitHub token** (API key)", then what for: "in `gh release create …`", "in the Password field on accounts.example.com", "to sign in to example.com as al***", and the agent's own why, quoted and marked as the agent's | **Deny**, **Allow once**, **Always on example.com** (where the Vault spec allows one) |
| A rotation step | "**Builder** wants to rotate **Stripe key**: run `set`" (or `verify`), with the command | **Deny**, **Allow** |
| A request for a new entry | "**Builder** asks you to add **Postgres URL** (API key): <why>" | **Add it** opens the kit's form for that kind inline in the card (label and kind filled in, the secret parts masked); **Not now** |

The agent's name is the pane's agent, and for a request from elsewhere (§8) the agent's `name@host`. An entry is named only by its label and kind.

### 4.3 The countdown

- For an `api-key`, `login` or `note` entry, and every rotation step, the card shows "**Goes ahead in 0:30** unless you deny", counting down, with **Deny** first.
- For an entry of the Personal or Cards sections (`ssn`, `itin`, `tin`, `ein`, `date-of-birth`, `passport`, `drivers-licence`, `other-id`, `card`, `bank-account`), the card shows "**Waiting for you**: ID and card details are never used without a yes", and no countdown. It expires as denied at the request's own timeout (2 minutes for a command, 10 for a browser fill).
- A request for a new entry has no countdown: silence can't supply a value. It waits as today (an hour), and "Not now" declines it.
- The deadline is srv's (§5), so the card only shows it.
- While the person's pointer or keyboard focus is in the card, the countdown holds, as the question card's does; it resumes 15 seconds after the last activity (decision 1).

### 4.4 Focus and keys

The card never takes the focus: the person typing in the composer keeps typing, and a key pressed there never answers a card. Keys answer a card only when the focus is in it (Tab reaches it like any panel); Enter presses the focused button, never "the default". A request form's fields take the focus only when the person clicks **Add it**.

## 5. srv: one deadline, one record, one answer

- **Each request records its pane** (`block_id`) and, when known, the agent's machine, set where the request is made (§2's "which pane asked"), so the card and the attention surfaces find it.
- **srv owns the deadline**: `deadline_ms` on each pending use, 30 s after it is asked (§4.3's kinds) or the request's own timeout. It runs whether or not any window shows the card, a pane is mounted, or the window is minimized.
- **Silence approves only what may.** When the 30 s pass with no answer, a use of an `api-key`, `login` or `note` entry, or a rotation step, is approved **once**. Silence never grants "Always". A use of a Personal or Cards entry is never approved by time: it expires as denied.
- **The answer** still comes only through the host's route (`answer_use`, a host token), now from the card; Deny on a card answers that request only.
- **Hold**: the window tells srv (host route) when the person's activity holds a countdown; srv extends that request's deadline to 15 s past the last hold, never beyond the request's own outer timeout.
- **Limits**: 16 pending per agent and 64 overall, the oldest of that agent denied when it has too many, so one agent can't push out another's requests (today one shared 16).

### 5.1 What the agent sees

The agent's call still waits for the outcome, now at most 30 s for a countdown kind:
- approved by the person: the call goes ahead, as today;
- **approved by timeout**: it goes ahead, and the result says "no one answered in 30 s, so it went ahead";
- denied: "the person said no";
- expired (an ID or card entry nobody approved): "an ID or card entry is used only on a yes; ask the person";
- a request for a new entry answers `pending` with its id at once and `created` or `declined` later, as today.

While it waits, the agent's tool row shows "Waiting for your approval", and the card is the only place to answer.

### 5.2 The audit

Each outcome is an Activity line, as today, with who decided: "approved by you", "**approved: no answer in 30 s**", "denied by you", "expired". The Vault pane's Activity can filter the timeout approvals, so the person can see everything that went ahead while they were away.

## 6. When the pane isn't on screen (agent pane and app chrome: for AgentX's review)

Each pending card registers with the "waiting for you" registry (`waiting-for-you.ts`) as source `vault`, through srv's `userattention` event with the pane's id, so everything the question card gets, it gets:

| Where the pane is | What the person gets |
|---|---|
| On screen | The card, and the tone |
| Another tab | The tab's waiting badge, the tone, and the Swarm chip `vault` with its countdown ("0:24"), or "Waiting" for an ID or card entry |
| Another window, or the window minimized or behind another app | The OS notification: "**Builder** wants to use **GitHub token**: goes ahead in 30 s unless you deny" (or "needs your yes"). A click brings that window, tab and pane forward with the card. The taskbar badge. |
| Hibernated or not mounted | The same badges and notification; the card is there when the pane mounts. The countdown runs in srv regardless. |
| No pane (a parked agent, #6035) | Its **My agents** row shows the request in a line under its name, with **Deny** and **Open**; **Open** attaches the agent to a pane (as "Open it" does) with the card in it. The status bar's attention item counts it, and the notification's click opens My agents at that row. |
| No agent pane at all (a request from another machine, §8) | The Vault pane's **Requests** list (below) and the status bar's attention item; the notification opens the Vault pane |

The **Vault pane** gets a **Requests** list of every pending request, from every agent, with the same controls and countdowns: one place to see and answer them all. It is a list in the pane, not a modal.

**Sound.** The tone is the registry's waiting tone, with its settings (on or off, volume, the app's mute and Do Not Disturb), not a new sound. It loops from the first waiting request in a pane until its last is answered, so several requests play it once; a request in a second pane while the tone already plays doesn't restart it. While the pane is on screen and focused the tone plays as it does for a question card. Do Not Disturb silences it and holds the OS notification, as for questions; the countdown still runs (silence is silence).

## 7. Several at once

- **In one pane**: the head request shows in full; a line under it says "**2 more**: Stripe key (0:12), Passport (waiting)". Each request has its own deadline, counting down at the same time, so they don't wait in line; a later one can go ahead before the head. **Deny all from Builder** sits beside them.
- **Across panes**: each pane shows its own. The status bar's attention item counts them all, and the Vault pane's Requests list shows them together.
- **Repeats**: an agent asking again for the same entry and target while a request is pending joins it rather than adding a card (as requests for a new entry do today).

## 8. Container agents and other machines

- **Container agents** use the same routes; srv knows their pane from their verified identity, so their card shows in their pane on the host, the same.
- **Agents over SSH or WSL** can't use a Vault secret today (`shell_secrets.rs:251-255`); nothing changes.
- **Other machines**: an agent uses the Vault of the machine it runs on, so its card is in its own pane there. When an agent on another machine asks this machine's Vault for an entry only this machine holds (a shared Vault's LAN or Cloud tier, `SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md`), the request shows on **this** machine, the one whose Vault it is, in the Vault pane's Requests list and the status bar, naming `Builder@laptop`. The same 30-second rule applies; the asking machine's agent sees pending, then the outcome.

## 9. Anti-forgery

- **The card is drawn from srv's record**, not from the transcript: no agent output, tool result or markdown becomes a card. It is docked outside the scrolling transcript, so nothing the agent prints sits where a card does.
- **Agent markdown can't draw a control**: the sanitizer has no `<button>`, form or input. An image of a card is only a picture; clicking it does nothing.
- **Only the real control counts**: the answer goes through the host's route with its token; an agent's connection can't call it (`ctx.operator_only`, host token), and its `UI` query, click and keys refuse the card's `data-operator-only` (`query.js`).
- **The request form for a new entry is operator-only too**, as the prompt that offers it is: an agent must never type the value it asked for into the person's form.
- **The card names the agent from srv's record** (the verified identity that asked), never from text the agent supplied; the agent's why is shown quoted and labelled as the agent's words.

## 10. Other modals: which follow now, which later

The operator wants to steer away from modals in general. Every modal in the app today (24), by who opens it:

| Opened by | Modal | Where | Plan |
|---|---|---|---|
| An agent | Vault use (`UsePrompt`) | `frontend/app/view/vault/vault-use-prompts.tsx:37` | **Now** (this spec) |
| An agent | Vault request (`RequestPrompt`) | `frontend/app/view/vault/vault-request-prompts.tsx:20` | **Now** (this spec) |
| An agent | A question for text or a yes (`UserInputModal`) | `frontend/app/modals/userinputmodal.tsx:15` | **Next**: the same card in the agent's pane |
| An agent | Control a desktop app (`DesktopRequestPrompt`) | `frontend/app/block/desktop-requests.tsx:38` | **Next**: card in the agent's pane |
| An agent | Install a widget (`InstallRequestPrompt`) | `frontend/app/block/widget-install-requests.tsx:31` | **Next**: card in the agent's pane |
| An agent | Upgrade AgentMux (`UpgradeRequestPrompt`) | `frontend/app/block/upgrade-requests.tsx:32` | **Next**: card in the agent's pane, plus the status bar (it restarts everyone) |
| The browser | HTTP sign-in (`BrowserAuthModalPanel`) | `frontend/app/view/browser/components/BrowserAuthModal.tsx:24` | **Next**: a band in the browser pane, like its Approve banner |
| The system | Messages and config errors (`MessageModal`) | `frontend/app/modals/messagemodal.tsx:11` | **Later**: a toast or the status bar |
| The person | Closing a tab, window, pane or workspace, shutting an agent down (`TabCloseConfirmModal`, `WindowCloseConfirm`, `RemoveWindowConfirm`, `ReplacePaneConfirm`, `DeleteWorkspaceConfirm`, `ShutdownConfirm`) | `tab-close-confirm-modal.tsx:9`, `window-close-confirm.tsx:18,55`, `replace-pane-confirm.tsx:12`, `workspace-modals.tsx:126`, `shutdown-confirm.tsx:11` | **Later**: act at once with an Undo toast, or confirm inline where the action is |
| The person | Workspace name, picker and import preview | `workspace-modals.tsx:18,70,150` | **Later**: inline in the sidebar, or a pane |
| The person | Agent settings panels (seat, container limits, environment, bundles, bundle import) | `frontend/app/element/modal-dispatch.tsx:44-125` | **Later**: already pane-scoped; a side panel |
| The person | Command palette | `frontend/app/modals/command-palette.tsx:32` | **Keep**: a picker the person opens and dismisses at once |
| The person | Sign in with another account (`AnotherAccountLogin`) | `frontend/app/view/agent/failure/AnotherAccountLogin.tsx:13` | **Keep for now**: a sign-in the person started |

"Next" items each get their own task once this lands, reusing this card (§4) and §6's surfaces.

## 11. Decisions for the operator

1. **Does the countdown hold while the person is in the card?** Recommended: yes, as the question card does (it resumes 15 s after the last activity), so someone reading a long command isn't overruled mid-read.
2. **Silence never grants "Always".** Recommended: a timeout approves once only.
3. **Per-agent limits** (16 pending each, 64 overall), so one agent can't flush another's requests. Recommended: yes.
4. **The countdown for a parked agent's request**: it runs like any other. Recommended: yes; the My agents row and the notification show it.
5. **The Vault pane's Requests list** as the one place for every pending request. Recommended: yes.

## 12. Security

- **Silence now approves.** An API key, sign-in or note can be used by an agent 30 s after it asks if nobody answers: the operator's choice. What bounds it: one use, never "Always"; IDs and cards excluded; every timeout approval is an Activity line the person can filter; the agent still never holds the value (it goes into a command's environment, a form field or a sign-in, scrubbed from output as today).
- **No new route returns a value.** The card and its answer carry ids, labels and outcomes only.
- **Forgery** (§9): the card is chrome from srv's record, operator-only, answered only through the host's route.

## 13. Phases and who builds what

| Phase | What | Who |
|---|---|---|
| **C1** | srv: the pane on every request, `deadline_ms`, the 30-second rule by section, hold, per-agent limits, the agent's outcomes, Activity lines | Agent2-6 |
| **C2** | The card in `AgentBottomPanels` (use, rotation, request with its inline form), operator-only, kit `Button` and `Menu`; the `vault` source in the waiting registry and the Swarm chip | Agent2-6, reviewed by AgentX (pane) and Agent1 (kit) |
| **C3** | Out of sight: the notification text and click, the My agents row for parked agents, the status bar count, the Vault pane's Requests list; the two modals removed | Agent2-6, reviewed by AgentX |
| Next | The other agent-opened modals (§10), one task each | Their owners: AgentX (agent pane, desktop, user input), Clamk or QTest (upgrade), Korp (browser HTTP sign-in), Agent1 (widget install) |

One PR per phase.

**As built (C1).** `backend/vault/uses.rs` keeps each use's `block_id` (the asking agent's own pane), `deadline_ms`, `expires_ms` and `goes_ahead`, and decides it on silence itself; `server/vault_ask.rs` is the one wait every use goes through (fill, command, capture, sign-in, rotation), and writes the "approved: no answer in 30 s" Activity line. The host's `hold_use` holds a countdown. A new-entry request records its `block_id` too. Until C2's card lands, today's prompt shows srv's countdown ("Goes ahead in 0:30 unless you deny", or "Waiting for you" for an ID or card entry) and holds it while the person reads or acts in it, so nothing goes ahead unannounced.

**As built (C2).** `agent/components/vault-request-card/vault-request-cards.tsx` follows srv's lists of pending uses and requests in every window and keeps a card, through `paneRequests`, in the pane each names (`block_id`); a card goes when srv's list drops it. The use card and the window's prompt draw the same question (`vault/vault-use-question.tsx`), the card never taking the focus. **Add it** opens the Vault's entry form as a pane modal in the agent's pane (`openPaneModal`), rather than inline in the card; **Not now** declines. The tone, badges and notification are srv's attention for the agent's pane (`vault_ask` now names it rather than the browser pane), so a card adds no wait of its own. A request with no pane is still asked in the window, until C3. Not yet: "Deny all from <agent>" (§7). Also in C2: srv keeps a request's pane only while a window shows it (`vault_ask::shown_pane`), so a parked agent's request is asked where it can be answered; a Vault ask counts as the agent's own wait, so a pane in a tab not in front shows "Waiting for you"; the agent's words are cut to a length, the whole in a hover tip.

**As built (C3).** The two window modals are gone (`UsePrompt`, `RequestPrompt`); `startVaultRequestCards` now keeps srv's lists live in every window. The Vault pane's **Requests** list (`vault/vault-requests-list.tsx`) asks every pending use with the card's question, answers and countdown, lists every ask for a new entry, and offers "Deny all from <agent>" for an agent waiting on more than one; it is where a request with no pane on screen is answered. A parked agent's **My agents** row says what it waits on, with srv's countdown, **Deny** and **Answer**; Answer opens the Vault pane's list rather than attaching the agent as §6 says, because the request kept no pane for a new one to show it in. The status bar's **Vault requests** item counts requests with no pane on screen and opens the Vault pane; one asked as a card is counted by the agents' "waiting on you". The card offers "Deny all N from <agent>" too (§7). The notification's text says what silence does ("…: goes ahead in 30 s unless you deny", or "…: needs your yes"). Not yet: an OS notification and its click for a request with no pane (a request under an `app:` key plays the tone only), and the Swarm chip's countdown; both are follow-ups.

## 14. Tests

- srv: a use of an `api-key` with no answer is approved once after 30 s and its Activity line says so; a `passport` use is never approved by time and expires denied; a request for a new entry never approves itself; a hold moves the deadline but never past the outer timeout; per-agent limits deny that agent's oldest only; the agent's results read as §5.1.
- Card: shows the agent, label, kind and target, never a value; the countdown for API keys, none for IDs and cards; Deny answers one, Deny all answers that agent's; the card is `data-operator-only`; it never takes the composer's focus, and Enter in the composer doesn't answer it.
- Forgery: agent markdown with a button, a form or a look-alike block renders no control; an agent's `UI` click on the card is refused.
- Attention: a card in a background tab badges it and starts the tone once; a second card adds no second tone; the Swarm chip counts down; the notification's click opens the pane with the card; a parked agent's request shows on its My agents row; Do Not Disturb silences the tone and the countdown still runs.
