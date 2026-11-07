# Spec: when something waits on you, you can see what, where, and one click takes you to it

**Date:** 2026-11-05 (task #18210, build #18211). Revised the same day after the operator widened it (section 1).
**Status:** active — approved; PR 1 (the toast, the one list from the registry and srv's rows, the setting, the sound rule) in #6773. Remaining: the status-bar list and its count (3.6), the shutdown chime and `app:` asks as registry sources (3.8, 3.1), parked agents, Kanban and LAN groups, the 10 s close, Do Not Disturb, the optional recent rows (D7).
**Author:** Agent5@narko, for AgentX's lane
**Scope:** `frontend/app/notification/` (the waiting registry, the sound service, the toast stack), `frontend/app/statusbar/AgentCountsStatus.tsx`, `frontend/app/util/reveal-block.ts`. No new srv route; the build may add a field to an existing event (3.3).
**Related:** `SPEC_REVEAL_BLOCK_ONE_PATH_2026_09_27.md`, `SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md`, `SPEC_SWARM_OTHER_HOSTS_AND_CHANNELS_2026_10_02.md`, `REPORT_AGENT_ATTENTION_CTA_CONTRAST_AND_TONE_2026_10_10.md`, `SPEC_AGENT_COLOR_2026_08_08.md`

## 1. The ask

The operator, 2026-11-02: "i hear the beep, but it is hard to find the source. Can you add a toast in agentmux that provides a link to the running agent who needs attention?"

Widened 2026-11-05 (via AgentY and AgentX): "whenever there is something pending anywhere on the tiers (like what causes the beep sounds) i often find myself in situation where I hear the beeps but cannot find where its comming from. consider toast, or additional entry in status bar? we need something, consider best practices, when clicking on the notification it would bring whoever is asking for input to the foreground and selected".

So four things: every source of a beep or pending ask is covered (section 2); a toast and a persistent status-bar entry (3.2, 3.6); a click brings the asker forward and selects it, wherever it is (3.4); and a rule so a new beep cannot appear without a visible source (3.8).

## 2. Every source, and whether the registry has it

The beeps are the ones in `sounds.ts`: `agent.waiting.for.input` (a looping tone) and five one-shot ones (turn complete, error, interrupted, message accepted, rejected), plus the shutdown-pending chime and tool tones. The waiting tone is the only one that means "something is asked of you". Every ask that beeps goes through one function, `startWaitingForYou` in `waiting-for-you.ts`; the sound service, the OS notification bridge and the pane's "Waiting for you" tab state all listen to its two events.

What srv knows is wider than what one window's registry knows. srv's `agentruntime` rows (`state: waiting_on_user`, with `block_id`, name and colour) cover every agent on this machine, in every window and workspace, with or without a pane: the state is the controller's parked request or the pane's `term:awaiting_user` meta (`agent_state.rs`). srv's router also detects a question from the agent's output with no pane mounted (`stdout_reader.rs`) and sends the OS notification for it. The status bar's "N waiting on you" (`AgentCountsStatus`) already reads those rows.

| Source | In the registry (beeps) | Visible elsewhere today | What joins it |
|---|---|---|---|
| A question (AskUserQuestion) | Yes (`useAgentQuestions`) | agent state, tab, Swarm row, OS notification | Toast, list row |
| A tool permission | Yes (`useAgentDecisions`) | the same | the same |
| Pane requests: cards and modals (e.g. Vault cards, #17125) | Yes (`paneRequests.push` with `waits`) | the pane | the same, with no code on their side |
| srv `userattention` kinds `browser`, `consent`, `widget`, `vault` and the upgrade and desktop-control asks | Yes (`applyUserAttention`) | the pane's banner or card | the same |
| An ask that belongs to no pane (`app:` key) | Yes, but beeps only while no window is in front | nothing | list row only: it has no pane, so its action is "Open" (3.4) |
| The shutdown-pending chime (Keep running within 30 s) | No (`playShutdownPendingTone`, own banner and OS toast) | banner, OS toast | Becomes a registry source (3.8), so it gets the toast and the row |
| An agent in another window | Yes, in that window only | its tab, if you look there | The status-bar list reads srv's rows, which every window has |
| An agent in a workspace no window shows | No: no pane is mounted, so the registry never hears it. Questions still raise the OS notification; the status bar count includes the agent | OS notification, status bar count | List row from srv's rows; the click opens the workspace (3.4) |
| A parked agent (no pane) | No | status bar count, OS notification | List row; action "Open" |
| Kanban "Needs you" tasks | No: they make no sound (`useKanbanNeeds`) | the Kanban's widget-bar badge | List row group, "3 tasks need you", opens the Kanban on that column. No toast, no tone: it is not a beep. |
| An agent on another LAN host | No: no beep here | the Swarm's host sections show its state (`swarm_remote.rs`, Phase 4) | List row from the Swarm's remote state, "Korp@starpower waits"; the action opens the Swarm at that agent. No toast: the other machine beeps for itself. |
| Turn complete, error, interrupted, accepted, rejected | No (not asks) | each flashes its pane's tab (`emitActivityFlash`), in that pane's window | Optional "recent" rows (3.7) |

## 3. Decisions

### 3.1 One entry point, and one list

The toast and the status-bar list are two views of one list, built from the same three feeds and merged by agent (pane):

1. this window's registry (full text, instant, includes `app:` asks and pane requests);
2. srv's `agentruntime` rows with `state: waiting_on_user` (every window, every workspace, parked agents);
3. srv's open `userattention` requests, which the registry already receives (`NotifyAttentionCommand`), for the other windows;

plus the two list-only groups from section 2 (Kanban "Needs you", LAN hosts). A row has the agent's name in its pane colour (the row's `color`, the shared palette), what it waits for in a few words, and how long. `startWaitingForYou` gains an optional `what` ("asks to approve a Bash command", "wants auto-approve on github.com"); callers that give none get a default from the source name. Its `text` stays what the OS notification and tab show.

### 3.2 The toast

One line per request: the agent's name, what it wants, **Go to agent**. It is the quiet "new" signal: shown when an item enters the list, not for items already there when the window opens. It is rewritten in place, not stacked: with N waiting it reads "N agents need you" with a link per agent (newest first, at most five, then "and M more" which opens the list). It uses the existing toast stack, so it is `role=status`, never takes focus or keyboard input, and respects `data-toast-avoid`.

It shows in the window that is in front. It shows for an agent in another window or workspace too, with "in <window name>" (or "in <workspace>"). When no AgentMux window is in front, the OS notification is the signal (it exists already) and no toast is added, so there is never an OS toast and an in-app toast for one request.

### 3.3 Where each window gets the list

Its own registry for its own panes, and `agentruntime` rows for the rest, as in 3.1. This needs no new route. The build confirms that a row's `waiting_on_user` covers every registry source; where it does not (a browser hand-off, a consent or a widget ask is the agent's own wait only for `vault` today, `AGENT_WAITS`), feed 3 covers it. If a source is in neither, the build adds it to srv's `userattention` rather than to a third feed.

### 3.4 The click

The action is `revealBlock(blockId, { source: "attention" })`, the one reveal path. It switches the tab, selects a background member of a pane's stack, un-magnifies another pane, focuses the pane and puts the caret in the request. It also restores a minimized pane and expands a collapsed one: if `revealBlock` does not cover those two today, the build adds them there, not in the toast.

For a pane in another window it asks that window (`block.reveal`, `await_ack`) and raises it. For a pane in a workspace no window shows, #6325 already handles it: `block.reveal` is sent with `open_if_closed`, srv parks the pane for 30 s and the caller opens a window on that workspace (`openWorkspaceInNewWindow`), which takes the pane up when it loads, so the click ends with that window in front and the pane selected. The build adds a test of this from the toast (today it is tested from the OS notification's click) and one more for a cold window that loads after the 30 s are up, which must say so ("couldn't open it in time") and keep the row. A parked agent has no pane: its action is "Open", which calls `openAgentInActiveTab({ agent_id, name })` from `agent-runtime-live.ts`, the one path every Open uses (My agents, Swarm, picker, palette; Agent1's pointer). AgentX's rule in #6588 (not merged when this was written) reopens the agent's workspace in a window with the pane focused when the workspace still exists, else opens it in the active tab. It answers a refusal string or nothing, so the row can say why it failed. The request shows in the pane that opens. An `app:` ask has no pane: its action raises the window that shows it and opens its card.

### 3.5 It clears itself

It is driven by state, not a timer. When a request is answered anywhere (this window, another window, the OS notification, the agent's own tool call finishing), the registry's `waiting-ended`, the `agentruntime` row leaving `waiting_on_user`, or the `userattention` end removes it. Closing the toast with its X does not answer the request and does not bring it back; a new request does.

### 3.6 The status bar: persistent, with a list

The existing entry, "N running, M waiting on you" (`AgentCountsStatus`), becomes the home of the list. It stays visible as long as anything waits (today it is hidden when no agent runs, and it only counts agents). Clicking it opens a list of rows: the agent's name in its colour, what it waits for, how long, and **Go**, plus the Kanban and LAN groups. Clicking a row runs 3.4. The count includes the Kanban's Needs-you tasks in its own colour-coded part ("2 agents · 3 tasks"), so the number answers "is anything pending anywhere".

This replaces the earlier idea of collapsing the toast to a counter after 10 s: the entry is always there, so the toast can go away without the request being lost. The toast stays while the request is new (about 10 s, or until the pointer or the request leaves), then it closes and the entry keeps the row. Do Not Disturb and the app mute (`notify:pause`) hide the toast and not the entry: the entry is a place you look, not an interruption.

### 3.7 Best practice, and what we take

| Product | What it does | Take |
|---|---|---|
| OS notification centres (Windows Action Center, macOS Notification Center) | A banner now, a list you can open later, a count on the icon; a click activates the app and the item | Banner plus a list: the toast plus the entry; the count is the entry's |
| VS Code | Toasts that time out into a bell with a count in the status bar; a list; actions on the toast; Do Not Disturb | The closest model: toast plus status-bar entry plus list, DND hides the toast but not the entry |
| Slack | Unread badges on each channel, an Activity list, a click lands on the message | The badge on the source (we have it: the pane's tab and the Swarm row) and the central list |
| Chrome ("this tab is playing audio") | The tab that makes the sound is marked with a speaker | The answer to "which one is beeping" is to mark the source: the tab flash and the "Waiting for you" tab state already do, in the source's own window |

(From general knowledge of these products; not re-checked today.) What AgentMux lacks is the central list and the cross-window pointer, which sections 3.2 and 3.6 add. Optional (D7): the one-shot sounds (turn complete, error, interrupted) also leave a quiet "recent" row in the list for five minutes, so "who just beeped" has an answer when the pane is in another window.

### 3.8 The rule: nothing beeps without a visible source

- **The registry is the only way to start the waiting tone.** The sound service starts it only from the pane's `waiting-for-input` event, which only the registry fires. A test fails if a toast and list row are missing for any pane the sound service is playing the waiting tone for (both read `isWaitingForYou`).
- **The shutdown-pending chime becomes a registry source** (`source: "shutdown"`), so it gets the toast and the row. It keeps its own banner and OS toast.
- **Every sound is classified.** `SOUNDS` gets a required `source: "ask" | "event"` field, so a new `SoundId` does not compile without one. An `ask` sound plays only from the registry. An `event` sound must carry a `sourceBlockId` (it flashes that pane's tab); the sound service refuses to play one without it, and a test covers that.
- **A tone without a pane** (an `app:` ask) has a row, because the row is built from the registry, not from the pane.
- **What no test can cover** is a sound added outside the sound service (`new Audio`, an `AudioContext`). A lint rule in the repo's existing grep gates forbids creating audio outside `notification/sound/` (voice input is allowed by path), so a new beep has to come through the service and its classification.

### 3.9 A setting

`notify:waiting:toast`, default on, in Settings → Sounds beside `notify:waiting:browser`. Turning the master sound switch off does not turn the toast off: a person who muted the beep needs the toast more.

## 4. Decisions for the operator

| # | Question | Recommendation |
|---|---|---|
| D1 | One grouped toast, not a stack (3.2) | Yes |
| D2 | The toast closes after about 10 s because the status-bar entry is persistent (3.6). Changed from the first draft, which collapsed it to a counter | Yes |
| D3 | The toast shows for an agent in another window or workspace, not only the window holding the pane (3.2) | Yes |
| D4 | No in-app toast when no AgentMux window is in front: the OS notification covers it (3.2) | Yes |
| D5 | The toast ignores the master sound switch; Do Not Disturb and the app mute hide the toast, not the entry (3.9, 3.6) | Yes |
| D6 | Limits, sign-in and the spawn banner get no toast: they make no sound today (section 2). If you want them as attention, they join the registry first and then get the beep and the toast together | Yes |
| D7 | Turn complete, error and interrupted leave a "recent" row for five minutes (3.7) | Yes, if it is cheap; cut it first if not |
| D8 | Kanban Needs-you and other hosts' waiting agents are in the list but never toast or beep (section 2) | Yes |

## 5. Build plan (task #18211)

One PR, or two if the status-bar list is large (the toast and the rule first, then the list).

1. `waiting-for-you.ts`: the optional `what`; callers in `useAgentQuestions`, `useAgentDecisions`, `paneRequests`, `applyUserAttention` give theirs; the shutdown-pending chime becomes a source.
2. `attention-list.ts` (new, next to `os-notify-bridge.ts`): the merged list of 3.1, deduped by pane.
3. `attention-toast.ts` (new): the toast of 3.2, driven by the list's new items.
4. `AgentCountsStatus.tsx`: stays visible while anything waits; the popover list of 3.6.
5. `revealBlock`: confirm a minimized pane is restored and a collapsed one expanded, add it there if not; the closed-workspace tests of 3.4.
6. `sounds.ts` `source` field, the sound service's refusal, the grep gate of 3.8.
7. The setting of 3.9.
8. Tests: one request, several grouped, an answer in another window clears it, the X does not clear the request, no toast when no window is in front, a closed workspace, a parked agent, the parity test of 3.8, and the classification test.
9. A look check on macOS and Linux before merge (a visual change; the workspace rule).

## 6. What the first PR built, and where it differs

PR 1 (#6773) builds the toast, the merged list (this window's registry plus srv's `waiting_on_user` rows), the setting (3.9) and the sound rule (3.8) except the shutdown chime. Where it differs from the text above, until the list ships:

- **The toast stays while the request is open** (a persistent toast with a pin, closed by its X or when the list empties). The 10 s close of 3.6 needs the status-bar entry to keep the row, which is PR 2; closing it earlier now would lose the request.
- **The pane the person is looking at gets no toast** (their own request is in front of them), and for a pane in another window the toast says "in another window" (srv's rows carry no window name).
- **What was waiting when the window opened is not new**, for the first 4 s: srv's rows arrive a moment after the registry.
- **The toast keeps clear of the places a request is answered:** the question panel, the permission dialog and the request dock carry `data-toast-avoid`, so the stack moves above them. The agent's name is drawn in the palette's identity colour for the theme (readable on light and dark), and a request that leaves the list and returns within 5 s (srv's rows flicker while another window reveals a pane) is the same request, not a new toast. A request that arrives while its pane is the one in front is shown once the person looks away.
- `revealBlock` already restores a minimized pane (there is no separate collapsed pane state), so the click needed no change there.
- The sound rule is `SOUNDS[id].source` (`ask` or `event`), refused from the event bus when wrong (`sounds.ts`, `sound-service.ts`), and `scripts/check-audio-creators.mjs` (a CI gate) forbids `new Audio` and `new AudioContext` outside `notification/sound/` (voice input's two files excepted).
