# Spec: a call that waits for the person returns "waiting" and frees the agent's turn

**Status:** active (first part built 2026-11-12, see "What was built" below). Owner: Korp; the waiting state and the wake-up note are Manoz's lane (agent-pane lifecycle). Task #47395 (this spec), #47396 (the build).
**Operator:** "often when a agent prompts for a browser permission dialog, i have quesitons but the agent is stuck in a progress bar waiting for my answer. in those cases, the agent should eexit progress, as he's waiting, so I can also ask questions that he can respond to. (without having to force it with a dbl esc)".
**Checked against:** `origin/main` @ `d2e371838`: `browser_attention.rs`, `browser_click_approval.rs`, `browser_dialog.rs`, `browser_allowlist.rs`, `user_attention.rs`, `browser_tool.rs` (`handoff_status`).

## 1. How it waits today

A Browser call that needs the person (approval of a committing click, a hand-off, an allowed-sites question, `request_auto_approve`, a first-use sign-in approval, a page's confirm box) is one open tool call. Behind it, srv's `browser_attention::ask` shows the banner, holds the pane locked and waits up to 10 minutes; the answer arrives from the host (`/api/v1/host/browser_attention`), and the same call then makes the click. The agent's turn is running the whole time, so whatever the person types waits until the call returns, or they interrupt the turn (double Esc), which kills the call and, for a hand-off, abandons it.

## 2. The behaviour

1. **Grace, then "waiting".** The call waits as today for a short grace (8 s: the common quick Allow still returns the finished result). If the question is still open, it returns at once `waiting for the person` with `{request: "<id>", kind, question, pane, expires}`. The tool text tells the agent: say in one line what you are waiting for, end your turn, do not call again, do not re-ask.
2. **The agent shows as waiting for them.** While a request of its is open, srv reports the agent as `waiting_on_user` with the question as the reason (the Swarm row, the pane's waiting tone and the OS notification already exist: `Asking`). The person types to the agent and gets an answer in the normal way; the banner stays up.
3. **Their answer in the pane carries the action out.** srv holds, with the request, the action to take: for a click, the checked request (ref, backend id, the page address, the submission fingerprint the host computed); for a hand-off, nothing to do but report. On Allow / Done, srv re-checks and performs it through the same path as today (`approve_click` → the host's `check_click` with the fingerprint, then the click), and then wakes the agent (§4).
4. **Outcome note.** The agent gets an `[AgentMux]` note that starts a turn: `approved and done` (with where the page is now, as a click answers today), `declined`, `timed out` (10 minutes, unchanged), `void` (the page changed, the person took the pane over, or it was paused: nothing was done), or `failed: <why>` (the click was refused after approval, e.g. the form changed). No polling. `Browser request_status <id>` (the shape of `handoff_status`) answers for a session that restarted, so the note is a convenience, not the only record.
5. **Cancel.** `Browser cancel_request <id>` takes the question down. One open request per pane: a new request on a pane that has one is refused (the refusal names the open one), so replacing a question is cancel, then ask. While one is open, the agent's other actions on that pane are refused as today (`tools_paused`), but `request_status` and `cancel_request` work.

## 3. Protections that do not change

- **Only the click in the pane approves.** A chat message never answers a question; the agent cannot answer its own (the answer path stays host-only, `browser_attention.rs` header).
- **The approval is for that click on that page.** The stored action is the one the person saw: same ref and backend node, same page address, same fingerprint. The agent cannot change it after the fact, and srv re-checks the address and owner when it acts.
- **Nothing outlives its conditions.** A navigation, a take-over or a pause voids the request (the existing rules for the held call, kept); the 10-minute limit and the 30-second windows stay; a hand-off that was `abandoned` because the call ended becomes impossible, since the call no longer waits.
- **No retry loop.** A call on a pane with an open request returns `waiting for the person's answer to <id>; end your turn` instead of a second banner.

## 4. The shared primitive

Build it once in srv, not once per tool: `pending.rs` holds `Pending {id, agent, pane, kind, question, expires, action}` and one function, `wait_or_detach(grace, request)`, which every wait calls instead of waiting on the answer itself. It does three things: announces (`Asking`, so the waiting state and tone come for free), returns `waiting` after the grace, and on the answer runs the action and sends the outcome through the wake path. The wake path is Manoz's: a note from AgentMux that **starts a turn** (the ordinary `[AgentMux]` notes do not; ones that ask something do) and the `waiting_on_user` state for an agent with an open request. The interface asked of that lane: `pending_for(agent) → Option<{kind, question}>` for the availability code, and `wake(agent, text)` for the note.

Order of adoption, all through the same function: Browser (click approval, hand-off, allowed-sites question, `request_auto_approve`, first-use sign-in, a page's confirm box) in the build; then Vault approvals, Terminal's first-use and secret prompts, Agents `spawn` / `create` cards, `UI` and `App` confirmations; Widget install already answers `pending` and moves to this shape.

**Harnesses:** the mechanism is a tool result plus a message, so it works for any agent that can receive a message: Claude Code first; an ACP or Codex agent gets the same `waiting` result and the same note as its next message. A model that ignores "end your turn" and calls again gets the refusal in §3, not a second question.

## 5. Tool text and tests

The `Browser` description, and the shared tool text for the other tools later, gain: `If a call answers "waiting for the person", say in one line what you wait for and end your turn. Do not call again and do not ask again: you are told the outcome, or call request_status.`

Tests: unit tests for the detach (a fast answer inside the grace returns the finished result; a slow one returns `waiting` and later the outcome), each outcome (approved and done, declined, timed out, void on navigation, void on take-over, failed after approval), replace and cancel, the one-per-pane rule, and the refusal on a second call. One live run in a `task dev` build with a local page: a form whose Send needs approval; the call returns waiting inside 10 s, the agent pane shows waiting for the person, a typed question is answered, Allow in the pane sends the form once and wakes the agent with the page address, a navigation before Allow voids it and nothing is sent.

## 6. Questions

1. **Grace length:** 8 s. Shorter makes every approval a two-step; longer keeps the agent stuck for the common case. OK?
2. **Take-over while a request is open:** voids it today; keep, and the note says `void: you took the pane over`.
3. **Manoz:** the wake note and the waiting state are his lane; agree the two calls above with him before the build (I will message him with this spec once it is approved).

## 7. What was built

The first build (task #47396) moves two waits onto the shared shape, all in srv (`browser_waiting.rs`, `browser_click_approval.rs`, `ui_handlers.rs`) and mcp (`browser_waiting.rs`):

- **Committing click approval** (`act`, and Enter or Space taken as a click) and **`handoff`** return `waiting for the person` after the grace (8 s, `AGENTMUX_BROWSER_WAIT_GRACE_MS` overrides it). A task of srv's keeps the banner's request, makes the approved click through the same checked path (the host's fingerprint check) and tells the agent the outcome with `deliver_system_note`, the path the upgrade "carry on" note already uses to start a turn. The agent shows as waiting for the person through the existing `Asking` attention state, so no change to the availability code was needed.
- **Outcomes:** approved and done (with where the page is now), declined, timed out, void (the person took the pane over), failed (the click was refused after approval, e.g. the form changed). A request the agent took down itself reads as cancelled and sends no note.
- **`request_status` and `cancel_request`** (by `request`, or `pane` for the newest): in-memory records, a day, the newest 300; an agent reads and cancels only its own.
- **While a request is open** the agent's other Browser calls on that pane are refused with the request's id and "end your turn".
- **Replace** is cancel then ask (question 5 above).

Not built yet, same function to adopt: the "page wants to leave its allowed sites" question, `request_auto_approve`, a sign-in's first-use approval, a page's confirm box, the terminal line approval, the take-over question, and the other tools' waits (Vault, Terminal, Agents spawn, UI and App confirmations). A page navigation voids an open approval through the existing cancel rules; it reads as void when the person took the pane over and as declined otherwise.
