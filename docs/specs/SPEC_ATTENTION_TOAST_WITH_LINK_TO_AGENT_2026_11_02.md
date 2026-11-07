# Spec: a toast when an agent needs you, with a link that takes you to it (2026-11-02)

**Status:** draft, task #18210; the build is #18211.
**Owner:** Agent1. **Lane:** AgentX (agent pane). Others on the same tone: Agent2-6 (Vault cards, #17125) and Korp (browser hand-off banners).

The operator, 2026-11-02: "we need to make it easier to id when an agent needs attention. i hear the beep, but it is hard to find the source. Can you add a toast in agentmux that provides a link to the running agent who needs attention?"

## 1. What there is today (read from the code)

There is one attention beep: the looping `agent.waiting.for.input` tone. It starts on a pane's `waiting-for-input` event and stops on `waiting-ended`
(`notification/sound/sound-service.ts`), and only `notification/waiting-for-you.ts` fires those two events. A pane is waiting from its first source to its last. The sources are:

| Source | Where it registers |
|---|---|
| a question (AskUserQuestion) | `useAgentQuestions` |
| a tool permission | `useAgentDecisions` |
| browser hand-off and approval, auto-approve banner | srv `Asking::start`, kind `browser` (`browser_attention.rs`, `browser_vault_fill.rs`) |
| SSH and host consent, shell secrets | srv, kind `consent` (`connections.rs`, `shell_secrets.rs`) |
| Vault requests | srv, `vault_handlers.rs` |
| widget install and widget agent prompts | srv, kind `widget` (`widget_agent_handlers.rs`) |
| upgrade request, desktop control | srv (`upgrade_request.rs`, `desktop.rs`) |

srv announces them as `userattention` events and the registry applies them (`applyUserAttention`). A request with no pane is kept under an `app:` key.
The other tones (turn complete, error, interrupted, message accepted or rejected) are not calls to action and get no toast. The stop countdown has its own dialog; the build checks that it is not also a registry source.

So the answer to "one helper, a new kind can't beep without a toast" is already half there: nothing beeps except through the registry. The toast goes in the same place.

## 2. Decisions

**D1. The toast is fired by the registry, not by its sources.** `startWaitingForYou` (first source on a pane) shows the toast and `endWaitingForYou` (last source) removes it, next to the `waiting-for-input` and `waiting-ended` events. No source gets code of its own, and a source added later gets both the beep and the toast by registering. A test asserts that every `waiting-for-input` event has a toast and the reverse.

**D2. What the toast says.** Its title is the agent's name, its icon dot in the agent's palette slot (the pane's identity colour, #6151). Its message is what it wants in a few words: the first source's `text`, first line only, cut to about 80 characters ("Allow Bash?", "Wants auto-approve on github.com"), never the agent's long text. More than one source on a pane adds "+N more". One toast per pane: while a pane waits, a new source updates that toast (same `id`), it does not stack.

**D3. "Go to agent" is `revealBlock`.** The action runs the one reveal path (`util/reveal-block.ts`, SPEC_REVEAL_BLOCK_ONE_PATH): it switches the tab, switches a multi-tab pane to the agent, un-magnifies a pane that hides it, focuses it and puts the caret in the request. For an agent in another window it raises that window (`revealInOtherWindow`, `raiseWindowById`). Two cases the build must check on the real path and fix in `revealBlock`, not around it: a pane minimized to its header (restore it) and a minimized or hidden window (restore, then raise). A **parked agent has no pane**: its toast action is "Open", the same call as the card's "Open it", and the request is shown in the pane that opens.

**D4. Which window shows it.** Each window's registry publishes its transitions on the event bus (`agent-waiting`: pane, window, agent, text, active). The window that has OS focus shows the toast; if none has focus, the window holding the pane shows it and the existing OS notification stays as it is. When the agent is in another window the toast adds "in <window name>" and its action raises that window. Several waiting agents at once collapse into one toast, "3 agents need you", each row its own link, the oldest first.

**D5. It does not get in the way.** It never takes focus or keyboard input (the kit toast already doesn't, `live: "status"`). It respects Do Not Disturb, the app mute and the tone's own settings (`notify:sounds:enabled`, `notify:sound:agent.waiting.for.input`): with the tone off the toast still shows, since the toast is the quiet way to find the agent, but DND and the app mute hide it. [Open: confirm with AgentX that the tone setting does not hide the toast.]

**D6. How long it stays.** It is `persistent` while the request is open and goes when the last source ends, answered in any window or closed. After about 10 seconds it collapses to a counter in the status bar ("2 waiting"), whose click opens the same list. [Recommended; a timeout would let the user miss it, and a toast that never goes would cover the window.]

**D7. Badges.** The Swarm row and the window tab already show a waiting chip and a flash from the same registry; no new badge in this build.

**D8. Remote and container agents.** Included where the attention event reaches this machine (a container agent's pane is a pane here; a LAN peer's request arrives as the peer's `userattention` and is shown with "on <machine>" and a link that opens the peer's pane through the existing Swarm path). If a peer's event doesn't carry what the toast needs, the toast says so and links to Swarm.

## 3. Build outline

1. `notification/attention-toast.ts`: `showAttentionToast(blockId)`, `clearAttentionToast(blockId)`, the multi-agent collapse, the status-bar counter. Uses `pushNotification` (id, `persistent`, `actions`, `onClose`) and `setNotificationAction`; no new toast component.
2. `waiting-for-you.ts` calls it on the first source and the last. The agent's name and slot come from the pane's block meta, as the Swarm row does.
3. The `agent-waiting` bus event for D4; one new field on `UserAttentionEvent` (`agent_id`, for the parked case) with its Rust and TS types regenerated.
4. Fix any gap D3 finds in `revealBlock`.
5. Tests: the registry-toast pairing, one toast per pane and the update, the collapse, DND and mute, the window rule, dismissal on an answer in another window; screenshots from my own `task dev` build with the fake agent (`task dev:seed`), light and dark.

## 4. Not in this build

A different tone, per-source sounds, a toast for the non-attention tones, and any change to the OS notification.
