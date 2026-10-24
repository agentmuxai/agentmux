# SPEC: One way to stop an agent (close it), and a shutdown toast with a countdown and a Keep running button

**Date:** 2026-10-23
**Author:** Agent1 (UI kit, theme), for Kanban #935; built in task #1705.
**Status:** active. Approved by the operator on 2026-10-23 with D1 to D7 as written, plus one change (section 3.11: the requesting agent's reason stays out of the toast and is cleaned for the banner). macOS toasts are out of this build; the macOS contract in 3.8 stays for the platform task that will build that backend. The spec is committed in the first PR of build #1705 (the contract); the toast follows in a second.
**Grounded in:** `origin/main` at `9dfcbfac6`. Everything below was read from that commit; the paths are in section 8.
**Related:** `SPEC_AGENT_SELF_QUIT_2026_09_24.md` (section 6.5 is the 15 s window; section 12.4 is how srv closes one tab), `SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md`, `SPEC_OS_NOTIFICATIONS_RICH_CONTENT_AND_CLICK_TO_PANE_2026_09_25.md`.

## 0. Summary

**The request (operator, 2026-10-23).** When an agent stops another agent there should be one option, and it should close the agent. The shutdown toast should show a countdown and let the user cancel from the toast.

**What happened.** An agent used `Stop` with action `agents` to retire another agent. That ended the session and left the pane open. Closing the pane took a second call (`Stop` with action `pane`). The Windows toast said "<who> asked to shut it down. Open it to keep it running." It had no countdown, and clicking it opened the pane; only the pane banner's Keep running button cancelled.

**The design, in four lines.**
1. `Stop` `agents` closes each target agent's tab: its session ends and its tab goes, the same as the user closing the tab. Only that agent's tab: other agents in the same pane stay. The conversation is kept, and the agent can be reopened. It uses the same 15 s window as today.
2. `Stop` `pane` is for panes that are not agents (browser, editor, files, and so on). It refuses an agent pane and says to use `agents`.
3. Stop-and-keep-the-pane stays in srv as an internal operation (the fork slug migration, teardown, `agent.stop`). No tool an agent can call offers it.
4. The toast says what will happen ("Closing QTimer in 15 s"). On Windows it has a progress bar and a seconds counter that tick once a second, and a Keep running button that cancels in srv directly, exactly as the banner does. On Linux it has the button and static text. On macOS there are no toasts at all today (section 3.8), so this spec defines the contract a macOS backend will use and builds nothing there.

### 0.1 Decisions I made that the operator may want to change

These are in the spec as written. Section 7 gives the reasoning for each.

| # | Decision | Alternative |
|---|---|---|
| D1 | `agents` refuses the caller's own block and says to use `self`. | Allow it, behind the window. |
| D2 | An agent that is already stopped but still has a pane gets the same 15 s window as a running one. | Close it at once. |
| D3 | `signal` and `staged` leave the `agents` schema. A call that still passes them works, and the result says they no longer apply. | Keep them. |
| D4 | The toast respects the master notification switch only. Today `notify:os:when = never` also silences it, against the code's own comment. | Leave as is. |
| D5 | Background tasks the agent started (dev servers) end with its tab. `agent:stopkeepsbackground` no longer applies to tool stops. | Keep them running. |
| D6 | The `window:confirmcloseworking` setting does not gate this. Its wording is reused (section 3.10). | Gate the window on it. |
| D7 | One toast per agent. | One grouped toast for a bulk stop. |

## 1. Current state

### 1.1 `Stop` `agents` stops the controller and nothing else

`crates/mcp/src/tools/stop_tool.rs` maps action `agents` to the `FleetBulkStop` handler. That posts to `/api/v1/fleet/bulk-stop` (`crates/srv/src/server/http_app_api.rs`), which calls `fleet_bulk_stop_with_override` (`crates/srv/src/server/app_api/fleet.rs`). For every target that has a running controller it opens a 15 s request with `pending_shutdown::Action::Stop`. When the window runs out, `run_action` calls `agent_io::stop_one_agent_block`, which ends the agent's process and keeps the block, its tab and its pane. That is the leftover pane.

Two more facts from the same code:
- A target with no running controller skips the window. It goes to `fleet_bulk_stop_impl`, which calls `stop_one_agent_block` at once. So the target of the second call in the story above would never have been closed by `agents` at all.
- `staged` is only used for those no-controller targets. A running agent's window ignores it, so for the agents that matter `staged` already does nothing.

### 1.2 `Stop` `pane` closes the whole pane, agent or not

`close_pane_as` (`crates/srv/src/server/app_api/pane.rs`) collects every member of the pane's tab stack and opens a window with `Action::ClosePane`. When it runs out, `close_pane::run` closes all of them. It does not look at what the pane holds. Closing the pane of one agent therefore also ends every other agent stacked in it. `close_pane_as` is shared with the `UI` tool's `run_command` `pane:close` (`crates/srv/src/server/ui_shortcuts.rs`), so that is a second way an agent can close an agent's pane.

### 1.3 The 15 s window already does most of the work

`crates/srv/src/sagas/pending_shutdown.rs` holds the window. Facts this spec relies on:
- `Action` has three variants ordered by `rank`: `Stop` (0), `SelfQuit` (1), `ClosePane` (2). A later request for the same block joins the open one and never restarts the clock; a request that does more takes over the action.
- `begin` and `keep` change the status under one lock, so exactly one of "the window ran out" and "the user pressed Keep running" wins. A test (`keep_and_the_window_running_out_never_both_win`) pins it.
- `SelfQuit` already does what the operator wants: `self_quit::run_with` calls `delete_block::run_with_policy(.., Policy::quit())`, which ends the agent and removes only that tab, keeping sibling tabs (SPEC_AGENT_SELF_QUIT section 12.4). Its only extra is that it waits up to 30 s for the agent's turn to end (`run_action`), which is right for an agent quitting itself and wrong for another agent's stop.
- `request` also raises the OS notification, with the body `"<by> asked to shut it down[: reason]. Open it to keep it running."`.

### 1.4 The toast

- Policy (`crates/srv/src/backend/notify/policy.rs`): `ShutdownPending` is Attention priority, delay 0, and exempt from pause, quiet hours and the rate limits ("the only warning that an agent stops in 15 s"). `should_show` suppresses it when the user is looking at that exact block (the banner is visible there), and otherwise shows it even when the app is focused, because Attention passes the unfocused rule. The title is `"{agent} will shut down in 15 s"`.
- Router (`router.rs`): `emit_fixed` builds the request. `ack(id, clicked)` retracts the toast and, for a click, publishes `notification:activate` so the window selects the pane. Nothing reads which button was pressed, because there are no buttons.
- Launcher (`crates/launcher/src/notify/`): `Notification` has `id, kind, priority, title, body, summary, tag`. `UserAction` is `Clicked` or `Dismissed`. The Windows toast skeleton is constant, with text inserted by `CreateTextNode` and `launch="agentmux-notify:<id>"`; the `Activated` handler ignores its arguments and uses the id its closure captured. The Linux backend sends one action, `default`. There is no macOS backend: `NullPresenter` logs and drops (`mod.rs`).
- The banner (`frontend/app/view/agent/shutdown/ShutdownPendingBanner.tsx`) counts down every 250 ms from `deadline_ms` and calls `AgentShutdownKeepCommand`.

## 2. Goals and non-goals

**Goals**
1. One agent-facing way to stop another agent, and it closes the agent's tab.
2. The same 15 s user window, with the same guarantees (exactly one winner; audited; repeat requests join).
3. A toast that says what will happen, counts down where the OS allows it, and cancels from the toast.
4. The toast and the banner behave as one: same wording, same cancel.

**Non-goals**
- Changing what the user's own closes do (tab ×, pane ×, `/quit`, the Swarm view's bulk stop). They have no window and are unchanged.
- A macOS toast backend. It needs a signed bundle and a Mac to verify, as the notification spec already says.
- Changing who may stop whom. Agents are trusted (SPEC_AGENT_SELF_QUIT section 0.1); the window is what protects the user.
- A "pause the countdown" or "extend" control.

## 3. Design

### 3.1 `Stop` `agents` closes the agent's tab

Add `Action::CloseTab` to `pending_shutdown.rs`:
- **What it does when the window runs out:** `self_quit::run_with(state, block_id, QuitOrigin::ExternalStop, Some(detail))`, with no turn wait. That is `delete_block::run_with_policy(Policy::quit())`: a graceful stop (interrupt the turn, let the process exit, force-kill at the deadline), release its work claims, stop its `Shell()` sessions, remove the block, and prune only its stack member from the layout. `QuitOrigin` gets a variant `ExternalStop` so the audit says who and how (`agent.quit` with origin `external_stop`).
- **`rank` and `merge`:** `CloseTab` ranks with `SelfQuit` (1). A pane close (2) still takes over a pending tab close. Two `CloseTab` requests for the same block fold together. `covers` is the one block, so a tab close never covers a sibling tab, and two tabs of one pane have independent windows.
- **The tool path:** `fleet_bulk_stop_with_override` becomes two functions. A new `fleet_close_agents_with_override(state, by, targets)` serves `/api/v1/fleet/bulk-stop` and opens `Action::CloseTab` per target. The existing function keeps its body under the name `fleet_stop_keep_pane_with_override` and is called only by the fork slug migration (`agent_handlers/fork_slugs.rs`), which stops agents and then restarts them in the same pane.
- **Targets with no running controller** (the stopped agent that still has a pane) go through the window like any other (D2). The toast's button then reads "Keep open" instead of "Keep running", because nothing is running; the label is one of two fixed strings chosen in srv.
- **Cross-channel targets** are still asked on their own instance (`forward_stop_pending`). The forward carries `"mode": "close"`. An instance that predates the field ignores it and stops only; its answer carries no `closed: true`, and the result says "stopped, its pane was left open (that AgentMux is older)" for that target.

### 3.2 `Stop` `pane` is for panes that are not agents

An **agent pane** is a pane with at least one member block whose view is `agent`, or `term` with an `agentId` in its meta (the orphan reaper's definition, `sagas/orphan_reaper.rs`, `Unplaced::is_agent`).

`close_pane_as` refuses an agent pane before it opens a window. It returns 409 with an `error` that names the agent members' block ids and says to use `Stop` with action `agents`. The refusal sits in `close_pane_as`, not in the MCP layer, so `UI` `run_command` `pane:close` gets it too and there is no second way around. The user's own pane × does not use `close_pane_as` and is unaffected. A pane that mixes an agent tab with a browser tab is refused as an agent pane; the agent closes the agent tab with `agents`, and the remaining non-agent tab can then be closed with `pane`.

The audit records the refusal (`pane.close`, success false) with the caller as its source.

### 3.3 Stop-and-keep-the-pane stays, internal only

Unchanged and not offered to agents: `agent.stop` and `stop_one_agent_block`, `Policy::stop` in `agent_teardown.rs`, the watchdog's stops, the Swarm view's `fleet.bulk-stop` RPC (the user's own, no window), the fork slug migration, and the internal cross-channel route `/agentmux/agent/stop` (still used by `fleet_bulk_stop_impl` for the user's bulk stop). `Action::Stop` stays in `pending_shutdown.rs` for the migration only, and a comment says so.

### 3.4 The tool contract

**Schema (`STOP_TOOL` in `crates/mcp/src/tool_schemas.rs`).** Actions stay `self`, `agents`, `pane`. In `inputSchema`, `signal` and `staged` are removed (D3). `block_id` is described as "pane: the pane to close, from Pane layout; not an agent's pane (use agents)". `targets` is described as "agents: the block_id values of the agents to close, from Agents list".

**Handler behaviour.** `stop_tool::call`:
- `agents` with a `targets` entry equal to the caller's own block is refused by srv (D1), in `failed`: `your own block: Stop agents closes other agents; to end your own session use Stop with action "self"`. The other targets in the call still proceed, so this is a per-target refusal in the result, not a refusal of the call. srv knows the caller's block from the signed `auth` the MCP layer sends.
- If `signal` or `staged` is present, the call still works and srv adds `"note": "signal and staged no longer apply: Stop agents closes the agent's tab"` to the result.
- `pane` keeps its existing check (no `block_id` is refused with a pointer to `self`).

**Description text for `Stop`** (the whole string, replacing today's; the `self` paragraph is unchanged and elided here):

> Ends work: your own session (self), other agents (agents) or a pane that is not an agent's (pane). Every call takes an action and is audited. Stopping an agent you don't own, or yourself without the user asking this turn, waits for a 15-second window in which the user can keep it running: expect the call to take at least 15 seconds; the result says whether the user kept it. self: [unchanged]. agents: the one way to stop another agent. It closes each target agent's tab, as if the user had closed it: the session ends and the tab goes. Only that agent's tab closes, so other agents in the same pane keep running. The agent's conversation is kept, and the user can reopen it from My Agents and resume. Targets are block_id values from Agents list; it refuses a block that is not an agent and your own block (use self). Every target gets its own 15-second window, all in parallel, and the user sees a toast with a countdown and a Keep running button. Returns JSON {succeeded, failed: [{id, error}...], kept_by_user}: succeeded means the tab is closed, a target the user kept is in kept_by_user and in failed. pane: close a pane that holds no agent (a browser, editor, files or similar), including one that is unresponsive, by block_id from Pane layout; it refuses a pane that holds an agent and tells you to use agents. Logged with your verified identity as the source. Closing only removes the pane from the layout.

**Results.** One JSON object, keys kept from today so existing callers still parse it:

| Key | Meaning |
|---|---|
| `succeeded` | block ids whose tab closed, or that were already gone |
| `failed` | `{id, error}`: kept by the user (`"the user kept it running"`), refused, or failed |
| `kept_by_user` | the subset of `failed` the user kept |
| `note` | present when `signal` or `staged` was passed, or for an older peer (3.1) |

`aborted_early` is dropped with `staged`. Refusals use `failed` with a pointer: `"not an agent pane: close it with Stop pane"`, `"your own block: use Stop self"`.

**Wire shape.** srv's 202 body keeps `status: "pending_user_override"`, `wait_at_least_ms`, `deadline_ms` and `pending: [{block_id, request_id}]`. The MCP polling in `tools/fleet.rs` and `tool_helpers.rs` (`merge_bulk_stop_outcomes`, `public_stop`) is reused. `public_stop` names are unchanged (`FleetBulkStop` still reads "Stop agents"); the joined-request messages in `close_pane_outcome` no longer need to tell the caller to "call Stop pane again to close it", because `agents` now closes.

### 3.5 Multi-tab panes

- A target block closes its own stack member only. `delete_block` prunes the one member (the reducer's "one tab of a stacked pane" rule), activates the neighbour, and takes the pane with it only when it was the last member. That is the behaviour #5071 fixed for the user's tab ×.
- Two agents stacked in one pane, both named in one call: two requests, two windows, two toasts, each with its own Keep running. Keeping one does not keep the other.
- A pane close already pending for the pane (from `Stop pane` on a non-agent, or the internal path) covers every member and takes over a tab close, as today. Because `pane` now refuses agent panes, this only happens for a mixed pane the user's own action started, which has no window; it is an edge, not a path.
- The Keep running button is per request. `keep` already accepts any block the request covers.

### 3.6 What the calling agent sees

- At once: the call blocks. The MCP layer polls `GET /api/v1/agent/shutdown/{request_id}` once a second, as now.
- After at most 15 s plus the close: the JSON in 3.4. A target that the user kept is in `kept_by_user`; the description tells the agent not to retry it.
- A target that was already gone (closed or stopped by the user meanwhile) is in `succeeded`: `superseded` maps to success, as it does today.
- `proceeding` (the window ran out and the close is under way) maps to success, as today.
- A failure in the close itself (`failed`) is reported with the error text from srv.

### 3.7 Window, audit and joins

Unchanged from section 6.5 of the self-quit spec: one window per block, repeat requests join and never restart the clock, the audit has one entry per requester when the request settles. The audit's `via` stays `FleetBulkStop` (the endpoint name); the saga's own `agent.quit` entry (origin `external_stop`) records the close, so the log shows both the ask and the act.

`PendingView` gains `effect: "stop" | "close_tab" | "close_pane"` (exported to TypeScript), so the banner and toast say "Closing" for the two closes and "Stopping" for the internal stop, without guessing from `via`.

### 3.8 The toast on each OS

**Text, all platforms** (app-controlled templates; the agent's name is already sanitised by `sanitize_name`):

| Slot | Text |
|---|---|
| Title | `Closing QTimer in 15 s` (the seconds are live on Windows, static elsewhere) |
| Body | `AgentY asked to close it. It stays in My Agents with its conversation.` The requesting agent's reason is **not** in the toast (3.11). Only the requester's name appears, cleaned by `sanitize_name`. The body is still dropped when `notify:os:preview` is `none`, like every body. |
| Button | `Keep running` (`Keep open` for an agent that is not running, 3.1) |

Clicking the toast body still opens the pane (the existing click-to-pane path). Clicking the button cancels and does not raise a window.

**Payload.** The srv `Notification` and the launcher's mirror gain two optional fields, each ignored by an older peer: `actions: [{ id: "keep", label }]` (fixed ids; the label is one of the two strings above) and `countdown_ms` (relative time left when shown, computed in the policy from the request's deadline, so clock skew never matters). `ShutdownInfo { deadline_ms, effect, running }` rides on the policy `Request`. Nothing agent-written is ever an action id or a label.

**Windows** (`crates/launcher/src/notify/windows.rs`).
- Skeleton, still constant: `<toast scenario="reminder" launch="agentmux-notify:<id>">`, a bound title, the body, a bound `<progress>`, and one `<action content="Keep running" arguments="keep" activationType="foreground"/>`. `scenario="reminder"` is what should keep the toast on screen for the whole window instead of sliding into the Action Center after about five seconds; **this is unverified and the first thing the live smoke test checks** (section 4).
- The title and progress are data-bound (`{title}`, `{progressValue}`, `{progressStatus}`). Their values go in `NotificationData` set on the toast before it is shown; they are plain strings and are never parsed as XML, which keeps the property the hostile-text test pins. The body and the button label are still inserted with `CreateTextNode`.
- **Countdown.** `Presenter` gains `update(tag, values)` with a default that does nothing. The shared loop in `notify/mod.rs` starts a one-second ticker for any notification with `countdown_ms`, from a local `Instant`, and calls `update` with the next title, a bar value that drains from 1 to 0, and a status such as `9 s left`. The Windows backend turns that into `notifier.UpdateWithTagAndGroup(data, tag, group)` with a rising sequence number, on its WinRT thread. The ticker stops when the toast is retracted or acked, when the update reports the toast is gone (the user swiped it away), or at zero, where the last update reads `Closing now`. It then waits for srv's retract. Fifteen updates per toast. I have not tested how Windows treats one update a second, and the smoke test checks it.
- **Button.** The `Activated` handler reads the event's arguments. `keep` becomes `UserAction::Keep(id)`; anything else is a body click as today. The id still comes from the handler's closure, never from the arguments string. The button handler does not call `AllowSetForegroundWindow`.
- The toast's `ExpirationTime` is set to the deadline plus two seconds, so a toast the user ignored does not linger in the Action Center after the agent is gone.
- Launcher restarts already clear every toast of the instance (`clear_all`, `update_endpoint`), so a button on a dead instance's toast cannot exist.

**Linux** (`notify/linux.rs`). The actions array becomes `["default", "Open", "keep", "<label>"]` for a notification that has an `actions` entry, only when the server advertises `actions` in `GetCapabilities`. `ActionInvoked("keep")` becomes `UserAction::Keep(id)`. The text is static: the title says "in 15 s" as of when it was shown, and the toast is `resident` with no expiry (already true for Attention) until srv retracts it. No live countdown: a replace-in-place every second re-pops the banner on several servers, and the progress hint is not honoured by all of them. A server without actions shows the toast with no button, and the body click and the banner still work.

**macOS.** There is no backend today (`NullPresenter`), so a macOS user gets no OS toast for any notification, this one included; the pane banner is all they see. The contract above is what a macOS presenter should implement when it exists: a `UNNotificationAction` "Keep running" in a category, static text, and the time-sensitive interruption level so Focus does not hide it. That is out of scope here and needs a Mac.

**Notifications off, app minimized, nobody present.**
- Master switch off (`notify:os:enabled`): no toast. The banner still shows in the pane. The shutdown still goes ahead after 15 s unless the user keeps it, as today.
- `notify:os:when = never` also suppresses the toast today, against the comment in `policy.rs` ("only the master switch silences it"). D4 makes the code match the comment.
- Minimized or unfocused app: the toast shows (no window is focused, so the unfocused rule passes), and the banner is behind it. A click or Keep running works from the launcher, which does not need a window.
- App focused on a different pane or tab: Attention passes the unfocused rule, so the toast shows.
- App focused on the pane itself: no toast (`looking_at`); the banner is in front of the user.
- No window open (background mode): the toast shows; the button works; a body click opens a window (existing `ClickPlan::OpenWindow`).
- OS Do Not Disturb or Focus Assist: the OS may hold the popup. The router does not bypass it (the notification spec forbids it, section 5.1). The window still runs; the shutdown proceeds after 15 s unless kept. Nothing in this spec changes that.
- Pause and quiet hours: no effect, as today.

### 3.9 Keep running from the toast, and the races

**Path.** Button press, then `UserAction::Keep(id)`, then `notify.ack` with `{ id, clicked: false, action: "keep" }` (one new optional field; the existing command carries it). In srv the handler looks up the live notification by id and takes the block from srv's own record, never from the request. If the notification's kind is `ShutdownPending`, it calls `pending_shutdown::active_for(block)` and then `pending_shutdown::keep(state, block, request_id)`: the same function the banner's RPC calls. Policy `Ack` retracts the toast. `notify.ack` answers `kept: "kept_by_user" | "too_late"` (optional field), which the launcher logs.

**Races.**
- *Keep at second 15.* `keep` and the timer both go through the one lock. If Keep wins, the status is `kept_by_user` and the timer's `begin` finds it no longer pending, so nothing closes. If the timer wins, `keep` returns `too_late`. There is no state where both happen. The existing 50-iteration test covers the pair; the new tests cover the toast path (section 4).
- *The window ends while the toast is still on screen.* `cleared(.., "proceeding")` retracts the toast at the moment the close begins. A button press after that finds no live notification, so `notify.ack` returns `known: false` and `kept` is absent. Nothing is cancelled and nothing breaks.
- *Keep pressed on the toast and the banner at once.* The first wins; the second returns `too_late` and the banner clears from the `cleared` event.
- *Two requests for one block.* They share one request and one toast (the group key is per block), so one Keep cancels both. A stronger request that takes over replaces the toast text.
- *The user is told when Keep was too late?* No. A user who pressed Keep at the last instant and still lost the agent sees the pane's shutdown log, and the agent can be reopened from My Agents with its conversation. A follow-up toast is more machinery than that gap warrants (Q7).

### 3.10 The banner, and one close/stop language

The banner changes only in wording and the effect: "AgentY asked to close QTimer: <cleaned reason>. Closing in 12 s." with the same Keep running button. The reason is cleaned in srv first (3.11). The reuse AgentX asked for from workspaces (`frontend/app/window/window-close.ts`) is the wording of what a close does and where the agent goes: "stays in My Agents, with its conversations" (its `removeWarningText`) is the sentence the toast body and banner use, so closing a window, removing a window and an agent closing an agent all say the same thing about what is kept.

I did not reuse the `window:confirmcloseworking` setting (D6). That setting decides whether the user's own close of a window asks first when an agent is mid-turn. The shutdown window is the opposite case: a close the user did not ask for, which exists so they can stop it. Gating it on a setting that turns confirmations off would let one "Don't ask again" tick remove the user's only chance to stop another agent closing their agent.

### 3.11 The requesting agent's reason: out of the toast, cleaned for the banner (added after the operator's review)

The `reason` on a stop request is free text written by another agent. Today it reaches the user raw: `request` puts it in the OS notification body (through `emit_fixed`, which does not redact) and in `PendingView.reason`, which the banner shows. This change closes both.

**Toast: the reason is left out.** The toast has a few seconds of attention and a button; the reason adds little to the decision and is the larger risk, since an OS toast also lands in the Action Center, on a lock screen and in screen shares, where the banner does not. The body names the requester and says what will happen, nothing else. `emit_fixed` for `ShutdownPending` is then given no agent text at all, only the app's own template.

**Banner: the reason is shown, cleaned once in srv.** `pending_shutdown::request` runs the reason through a new `clean_reason` before it is stored in `PendingView`, so the banner, the `agent:shutdown-pending` event (which is persisted and replayed), and the status endpoint all carry the cleaned text and never the raw. `clean_reason`:
1. If `crate::backend::reactive::sanitize::is_sensitive_message` matches the raw text (the same credential and destructive keyword list the jekt tier gate and `redact_body` use), the whole reason is replaced by the fixed text `Reason hidden: it looks sensitive.` Replaced wholesale, not masked, as `redact_body` does.
2. Otherwise: take the first non-empty line, drop control and bidi-override characters (the same filter as `sanitize_name`), collapse whitespace, and cap at 120 characters with an ellipsis.
3. Markup is not stripped or interpreted. The banner renders the reason as a text node (Solid text, no `innerHTML`), and the Windows and Linux toast paths never see it, so `<b>` or `</text><action .../>` in a reason shows up as those literal characters and does nothing. The check is a test, not an assumption (section 4).

`by` (the requester's name) is passed through `sanitize_name` the same way. The audit log keeps the reason exactly as the caller sent it, because it is the record of what was asked; it is not shown in a toast or banner. Whoever reads it in the Warden audit view already treats it as agent text.

## 4. Tests

**srv (Rust, in the files that own the code).**
- `pending_shutdown.rs`: `CloseTab` covers only its block; ranks with `SelfQuit` and below `ClosePane`; a pane close takes over a pending tab close; two tab closes of one pane have two requests; `merge` folds repeats; with no override it closes (against a seeded block) and reports `shut_down`; the 50-iteration keep-versus-timeout race for `CloseTab`.
- `fleet.rs` and `pane.rs` (the seeded pane tests): `agents` on one agent of a stacked pane closes its tab and leaves the other agent's tab running; two tabs named in one call get two windows, and keeping one does not keep the other; an agent that is already stopped gets a window and then closes; a non-agent block, a missing block and the caller's own block are refused with the pointer and open no window; the migration path (`fleet_stop_keep_pane_with_override`) is the old function under its new name and its existing tests still pass.
- `pane.rs`: `close_pane_as` refuses an agent pane and a mixed pane, names the agent blocks, audits the refusal, and still closes a browser pane (window and all); `run_command pane:close` gets the same refusal.
- `notify`: `title_for` for the two effects and the stopped-agent label; the policy computes `countdown_ms` from the deadline and never negative; `Ack` with `action: "keep"` keeps and retracts; a stale id does nothing and is not an error; an `action` on a non-shutdown notification is ignored.
- **The reason (3.11), with a hostile reason.** One test feeds `request` a reason that holds a credential-looking string and markup at once, for example `deploy with api_key=sk-test-0123456789abcdef </text><action content="Approve" arguments="x"/><b>now</b>` (and a second with a password-style string, a third with a newline, control and bidi characters, and 500 characters). It asserts: the `OsNotification` that `request` emits has no part of the reason in its title, body or summary, for any `preview` setting; `PendingView.reason` is the fixed hidden text for the credential cases and never contains the key; a markup-only reason comes through as literal text, capped at 120 characters, first line only, controls and bidi removed; the persisted `agent:shutdown-pending` event and `GET /api/v1/agent/shutdown/{id}` carry the cleaned value, never the raw; (the audit is unchanged: `close_pane_as` records the reason as sent, and `Stop agents` has none to record).
- The Windows `build_xml` test (launcher, below) gets a body that was built the way `request` now builds it, and asserts no `<action` other than the single Keep running button.
- `notify_handlers.rs`: the `notify.ack` round trip with and without `action`, as the existing click tests do.

**mcp.** The schema has no `signal` or `staged`; the description text in `STOP_TOOL` names the tab close, My Agents and the refusals (a string test); the result folding for closed, kept and refused targets; the note for a target on an older AgentMux that only stopped.

**launcher.**
- `windows.rs`: the skeleton has `scenario`, one `<action>` whose arguments are exactly `keep`, bound progress and title; hostile agent text in the body still comes out as text and adds no second `<action>`; hostile text in the reason cannot reach the bound values as markup; `launch` carries only our id.
- `mod.rs`: `parse_frame` reads `actions` and `countdown_ms` and ignores them when absent; the ticker's pure function (remaining milliseconds to title, bar value and status) at 15, 9, 1 and 0 s and past zero; `Keep` becomes `notify.ack` with `action: "keep"`.
- `linux.rs`: the actions array with and without the capability.

**Frontend.** `ShutdownPendingBanner.test.tsx`: the wording for `close_tab` and `stop`; the Keep running button still calls the RPC; a reason containing markup is rendered as literal text (no element is created from it), and the hidden-reason text renders as given. `shutdown-pending.test`-style cases for `pendingTitle` with the new `effect`.

**Live (cannot be unit-tested; done by the builder on narko and reported with screenshots).** In a `task dev` build, never the installed instance. Extend the existing ignored `live_toast_smoke` with a shutdown toast and check:
1. The toast stays on screen for 15 s with `scenario="reminder"`, or else what it does instead (this decides whether the reminder scenario stays).
2. One update per second ticks the title and bar smoothly, and Windows does not drop or throttle them.
3. The Keep running button cancels: the agent keeps running, the banner clears, the toast goes.
4. Letting it run out closes the tab, leaves a sibling agent's tab, and the toast is gone.
5. Swiping the toast away stops the ticker (no error in the launcher log), and the window still runs.
6. A real `Stop` `agents` call from an agent in the dev build returns the documented JSON after about 15 s.
Linux: the same checks 3, 4 and the no-actions fallback, by the Linux tester on a GNOME and a KDE or dunst session. macOS: none, no backend.

## 5. Rollout

- **Two PRs (build #1705)**, in agentmux, so each is reviewable. **PR 1** carries this spec and the contract: srv (`CloseTab`, the `Stop pane` refusal, the reason cleaning, `effect`), mcp, the banner wording and the Operator Config entry. **PR 2** is the toast: the payload fields, `Keep running`, the Windows countdown and the Linux button. PR 2 needs PR 1's `effect` and cleaned reason. They ship in one install, so version skew exists only between a dev build and an older peer instance on the same machine, which section 3.1 handles for cross-channel targets.
- **Changeset:** `minor`, because the tool contract changes (`pane` refuses what it accepted, `agents` closes, two parameters leave).
- **Operator Config and docs.** The MuxAPI entry in `crates/srv/operator-config-seed.json` describes `Stop` and is delivered to agents at launch; it changes in the same PR (`agents` closes, `pane` is for non-agent panes). The agentmux-docs page for the Stop tool gets its own PR with its own changeset; it is not part of this one.
- **Order inside the PR:** srv first (action, `CloseTab`, the refusal, the payload fields), then mcp, then launcher and frontend. The notification fields are optional in both directions, so each layer works with the old other layer: an old launcher ignores `actions` and `countdown_ms` and shows today's toast; an old srv sends neither and the new launcher shows no button.
- **Rollback:** reverting the PR restores today's behaviour; no data or settings migrate.
- **Not in the PR:** a macOS backend; grouping toasts (Q4); a follow-up toast for a too-late Keep (Q7).

## 6. Things I found that are outside the request

- `emit_fixed` is documented as taking "app-controlled fixed text", but the shutdown body includes the requesting agent's `reason`, which is agent-written. It bypasses `redact_body`, and the same raw reason is stored in the pane banner's data. Section 3.11 fixes both (the toast drops the reason; the banner gets it cleaned). The bug exists today. The same reason also reaches the user through `QuitSelf` and `ClosePane`, which go through the same `request`, so they are fixed together.
- `staged` has been a no-op for running agents since the window was added (1.1).
- The self-quit spec's section 6.5 says a `ClosePane` whose pane holds the caller's own block closes at once; the code gives it the window (a comment in `close_pane_as` says why). The spec text is stale; I have not edited it.

## 7. Open questions, with a recommendation each

- **Q1 (D1): may `agents` target the caller's own block?** Recommend no: "ending yourself" is `self`, which has the user-quote gate, and one way to stop an agent is the point. The window would protect the user anyway, so this is about clarity rather than safety.
- **Q2 (D2): does an already-stopped agent's pane get the window?** Recommend yes: one rule ("a close an agent asks for waits 15 s") is easier to state and to audit, and it matches `Stop pane` today. The alternative closes an idle pane at once; the user may be reading it.
- **Q3 (D3): drop `signal` and `staged`?** Recommend yes. `signal` has no meaning for a close (the teardown already force-kills at its deadline), and `staged` does nothing for running agents.
- **Q4 (D7): one toast per agent or a grouped one?** Recommend per agent for now. Twenty agents closed at once means twenty toasts, and Windows shows only a few at a time, so the rest go to the Action Center; the banner on each pane is unaffected. A grouped "Closing 5 agents in 12 s, Keep all running" is a good follow-up, but it needs a request group in `pending_shutdown`, which this change does not otherwise need.
- **Q5 (D4): should `notify:os:when = never` silence this toast?** Recommend no, matching the code's own comment, so only the master switch can. The user who set `never` chose not to be interrupted, but also chose it before any agent could close another.
- **Q6 (D5): background tasks end with the tab.** `Policy::stop` kept an agent's declared background tasks (dev servers) running by default (`agent:stopkeepsbackground`); `Policy::quit` ends them with the agent. Recommend accepting that: closing an agent that was retired should not leave its dev server running with no pane to show it, and it is what the user's own close does. If the operator disagrees, the close can pass `stop_background: false`, at the cost of a new teardown policy.
- **Q7: follow-up toast when Keep was pressed too late?** Recommend no (3.9).
- **Q8 (D6): reuse the `window:confirmcloseworking` setting?** AgentX suggested reusing the workspaces close wording and setting. I reuse the wording and not the setting, for the reason in 3.10. If AgentX still wants a setting here, it should be a new one that can only shorten the window, never remove it.

## 8. What I checked (paths, on `9dfcbfac6`)

- `crates/srv/src/sagas/pending_shutdown.rs`: `Action`, `rank`, `covers`, `merge`, `request_within`, `begin`, `keep`, `cleared`, `run_action`, and its tests.
- `crates/srv/src/server/app_api/fleet.rs`: `fleet_bulk_stop_impl`, `fleet_bulk_stop_with_override`, `forward_stop_pending`, `shared_channel_for`.
- `crates/srv/src/server/app_api/pane.rs`: `handle_close_pane`, `quit_self`, `close_pane_as`, `pending_body`, `handle_shutdown_status`.
- `crates/srv/src/server/http_app_api.rs`: `handle_fleet_bulk_stop`, and the cross-channel `stop-pending` and `stop` handlers.
- `crates/srv/src/server/ui_shortcuts.rs`: `pane:close` through `close_pane_as`.
- `crates/srv/src/sagas/self_quit.rs`, `delete_block.rs`, `close_pane.rs`, `agent_teardown.rs` (`Policy::quit`, `Policy::stop`, `Policy::close`), `orphan_reaper.rs` (`Unplaced::is_agent`).
- `crates/srv/src/server/agent_handlers/fork_slugs.rs`: the migration's use of the stop-and-keep path.
- `crates/srv/src/backend/notify/policy.rs` (kinds, titles, `should_show`, `paused`, `rate_limited`, `Ack`), `router.rs` (`emit_fixed`, `ack`, `block_labels`, `sanitize_name`, `redact_body`), `crates/srv/src/server/notify_handlers.rs` (`notify.ack`).
- `crates/mcp/src/tools/stop_tool.rs`, `tools/fleet.rs` (the `FleetBulkStop` branch), `tool_helpers.rs` (`public_stop`, `close_pane_outcome`, `merge_bulk_stop_outcomes`), `tool_schemas.rs` (`STOP_TOOL`).
- `crates/launcher/src/notify/mod.rs` (`Notification`, `UserAction`, `Presenter`, `session`, `carry_out_click`, `NullPresenter`), `windows.rs` (`build_xml`, `show`, the `Activated` and `Dismissed` handlers), `linux.rs` (`Notify` actions, `ActionInvoked`).
- `frontend/app/view/agent/shutdown/ShutdownPendingBanner.tsx` and `shutdown-pending.ts`; `frontend/app/window/window-close.ts` (the wording and setting AgentX pointed at).
- `crates/srv/operator-config-seed.json` (the `Stop` description agents receive).
- `docs/specs/SPEC_AGENT_SELF_QUIT_2026_09_24.md` (sections 0.1, 6.5, 12.4) and the git history for #5071 (`0954231dd`, the one-tab close fix).

I did not build or run anything. The Windows behaviours in 3.8 (the reminder scenario, once-a-second updates, button activation for an unpackaged app) are from the platform's documented API and the existing code, and are the first things section 4's live test confirms.
