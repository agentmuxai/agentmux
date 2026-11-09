# Spec: an agent starting agents is a notice in its pane with a veto, not an out-of-window approval

**Status:** active — approved by the operator 2026-11-08 (relayed by AgentY; all five §8 decisions as recommended, one correction on parked panes, §2); built under #29620. Camper@narko, 2026-11-08. Written against `origin/main` 514588176.

The operator's words (via AgentY): "we get a strange out-of-window modal whenever you ask to create an agent, its actually unusual, and should probably be removed. instead, we should be getting the notification inside the agent pane, via the standard panel (one used in browser work)". Then, correcting the first brief: "this violates an agentmux tenet which is agents have high autonomy. I notice the safeguards keep creeping in. Once again, in this case, agents can start other agents with a basic panel, and also an override". Then: "lets mod the functionality: on the first agent start, the panel shows, and you can accept and auto approve the spawning of future agents."

This spec replaces the first brief's "must keep" list (no agent can answer, no auto-accept, timeout = declined). Those were gates; the tenet says agents start agents. The one guard kept is the one the standing rule allows: nothing may forge the person's voice.

## 1. What exists

- **Three entry points ask the person today**, all through one function: `consent()` for one worker (`agent_spawn.rs:726`), the create path (`agent_spawn/create.rs:108`) and the many-workers path (`agent_spawn/many.rs:145`). Each builds a `kind: "consent"` question and calls `ask_consent()` (`agent_spawn.rs:756`) -> `connections::ask_user()` (`connections.rs:598`).
- **`ask_user` posts to the host** (`POST /agentmux/approval/ask`, crates/cef `ssh_approval.rs`), which opens a separate top-level "approval" window over the pane's window and waits up to two minutes. Anything but a click on Start it (closed, timeout, no host window connected) refuses the spawn with `NOT_ALLOWED`. Reason for the separate window, from its header: the main window outside a pane is reachable by every agent's `UIQuery`/`UIClick`, so an Approve button there could be clicked by an agent.
- **When the person is not asked.** `consent()` returns without asking when the person's own turn asked for the spawn (`self_quit::gate`, the `user_instruction`) and the route was not chosen by the agent (`route_asked` false). The question's text is built by srv from its own record (`consent_message`: who asks, the worker's name, model, placement, account or route line, who pays, the brief clipped to 400 characters).
- **The in-pane way already exists** and is the standard: `paneRequests.push(blockId, { source, Component, props, presentation: "card", deadlineMs, waits, onGone })` (`app/store/pane-requests.ts`). It draws a card above the agent pane's composer, is marked operator-only to agents' UI queries and keys, keeps a request for a hidden pane until the pane shows (no window fallback), gives the pane its "waiting for you" tone, tab badge, Swarm chip and OS notification, and supports a `deadlineMs` for a countdown. Templates: the Vault cards (`vault-request-cards.tsx`), the desktop-control card (`app/block/desktop-requests.tsx`: srv holds the request, an event feeds the card, the answer goes back through a proven path), the widget-install card, and the browser hand-off banner (`server/browser_attention.rs`).
- **A proven answer path exists without the host window**: an RPC marked `ctx.operator_only(..)` is accepted only from the AgentMux window, proven by the host, never from an agent (`rpc_types/misc.rs:241`; `sensitive.decide` in `sagas/sensitive_window.rs:160` is the same shape as what this needs).
- **Countdown model to copy:** the sensitive-operation window (`sagas/sensitive_window.rs`: a clock, "approve now" and "veto", proceeds at the deadline) and its levels (the OS-prompts report, decision 2: 15 s low, 30 s medium).

## 2. The design

**Starting an agent never waits for a click.** It runs, with the person told in the agent's pane and given a way to stop it in time, and a way to say "stop asking".

For each spawn (one worker, one created agent, or a batch of workers) srv decides by the requesting agent's state:

| Requester's state | What happens |
|---|---|
| **Auto-approved** (the person chose "Always" for this agent, §4) | The agent starts at once. The requester's pane gets a **non-blocking line**: "Posa started Korp-2 on Sonnet 5.5 (your Claude account). Task: ...". No card to answer, no wait. |
| **The person's own turn asked for it** and the agent chose no route (today's no-ask case) | The same: starts at once, same non-blocking line. |
| **Anyone else** (the first start by this agent, with no choice yet) | A **card** in the requester's pane with a countdown (default **30 s**, §5), the full facts, and three buttons: **Stop it** (veto), **Start now** (skip the wait), **Always allow this agent to start agents** (start now, and remember). If nobody acts, **it starts when the countdown ends**. |

- **Card contents (same information as today):** who asks, the worker's name, harness, model, route, who pays, the brief. Text is built by srv from its record and the agent's words are shown only as the quoted brief (clipped 400, as `consent_message` does), never as a title or a button label.
- **Where the request lives:** `paneRequests.push(leader.block_id, { source: "agent-notice", presentation: "card", deadlineMs, waits: false, summary, onGone })`. The pane's tone, tab badge and notification come from srv's attention event for the pane (`user_attention::Asking`, as the Vault and desktop cards do), so the card adds no wait of its own (built as `waits: false`). The non-blocking line is a separate component without buttons or countdown, closed on its own after 12 s or by its dismiss button.
- **Findable when the pane is hidden or parked:** a card is a "waiting for you" source, so the tone, tab badge, Swarm chip, OS notification and the statusbar attention list (#18210, #18211) list it; clicking the entry runs `revealBlock` (which now also plays the locate flash). The countdown runs in srv, so it does not pause while the pane is hidden: that is what the tone and notification are for.
- **Requester gone before the deadline:** a requester whose block was closed or deleted, or whose agent stopped, has nothing to start for, so the start is cancelled (`onGone` -> veto; srv also cancels when the leader's block no longer exists or its agent is stopped). Not a gate on autonomy: there is no requester left. **A parked agent is still running**, so it does not cancel: parked behaves like a hidden pane, the card waits in the attention list and the countdown still proceeds.
- **Veto, approve-now and Always are the person's only**, through one RPC, `agent.spawn.decide { request_id, choice: "veto" | "now" | "always" }`, `ctx.operator_only("only the person can stop, hurry or auto-approve an agent start")`. This is the one guard kept: an agent cannot forge "the operator said always" (a durable grant) or speak for the person. A **veto from an agent is not needed** (it can simply not ask), so it is also not accepted; the card is operator-only to UI queries and clicks as every pane request is.
- **No window connected (headless):** nobody can veto, so the countdown has no one to give it to: the start proceeds at once and is logged. (Today it refuses.) See decision 4.
- **What stays as it is:** the existing limits (a leader's cap on workers, name length, `INVALID_ARG`/`NOT_ALLOWED` refusals for those) are not part of the consent and are not touched. Nothing new is added.

## 3. What goes away

- `ask_consent()` and the three `consent` calls' use of `connections::ask_user` for spawn, and the "approval window, which no agent can answer" comment (`agent_spawn.rs:724`). The refusals "NOT_ALLOWED: the user said no / didn't answer" become one: "NOT_ALLOWED: the user stopped it" (veto) or the pane-gone cancel.
- The spawn use of the host's `/agentmux/approval/ask` route and the approval window (still used by the windows in §6 until each is decided).

## 4. Scope of "Always": a decision for the operator

"Accept and auto-approve future spawns" needs a scope. Options:

| Scope | Notes |
|---|---|
| **Per requesting agent (recommended)** | Stored on the agent's definition (a boolean next to its other settings, written only through the operator-only RPC). The grant belongs to the agent that was shown and accepted; a new agent asks once. Visible and revocable where the person already edits agents (its settings, and its pane menu: "Starts agents without asking ✓"). Mirrors "Always allow" for SSH (`remote/agent_access.rs`), except stored with the definition so it follows the agent and is not a separate file. |
| Per window tab or workspace | Fewer prompts for a team, but a grant outliving who earned it; harder to see. |
| Everyone (one Settings toggle: "Agents may start agents") | One click, no per-agent state; but the first card then cannot say whose grant it is. Offer it as a Settings switch regardless, default off (card shown), so a person who never wants the card can turn it on in one place. |

Recommend per agent, plus the global Settings switch.

## 5. Countdown length

Default **30 s**, the standard human-override window (the sensitive-operation and stop windows use 15 s for the lightest and 30 s for medium). A spawn spends money, so 30 s rather than 15. The first card's wait is the only wait. Read from one constant next to the sensitive-window table so the length is changed in one place.

## 6. The other out-of-window approval windows

Asked for: every one, with what it approves, and a per-window proposal against the same tenet (the only guard kept is one that stops a forged operator voice; any other is flagged, not carried over). "Not tied to a pane" says whether the request names an agent's pane, i.e. whether an in-pane card is possible.

| # | Window and where | What it approves | Tied to an agent's pane? | Proposal |
|---|---|---|---|---|
| 1 | **Agent start** (`agent_spawn*`, this spec) | An agent starting a worker, creating an agent, or starting workers; spends tokens | Yes (the requester's) | **Moves** to the pane: notice + countdown + veto + Always (this spec). |
| 2 | **SSH: agent access to a host** (`connections.rs:703`, `consent_for_ssh`; `SshApprovalWindow`) | Letting an agent log in to an SSH host with the person's keys and run a command; "Always allow" is stored per agent and host | Yes | **Moves** to the pane, same notice + countdown + veto + Always. It already has the per-agent "Always" store. Flag: it is a guard on an agent using the person's identity; the operator decides whether it keeps a first-time countdown or becomes a plain notice. |
| 3 | **SSH: add a host to the ssh config; remove AgentMux's helper from a host; end a session; install the helper** (`connections.rs:257, 358, 395, 448`) | The person's own click in the Remotes pane, confirmed once more | No (comes from the person's click in Remotes) | **Flag.** The confirmation exists so an agent calling the same RPC with srv's key cannot do it on its own. The tenet says do not carry that over for an agent; but these act on the person's machine and remote hosts and are their own click's confirmation. Recommend: becomes a pane modal in the Remotes pane (the #29002 rule: a pane's question is the pane's), answered by the person's own click; the agent-call path decided by the operator. |
| 4 | **SSH: ssh's own prompts** (password, passphrase, new host key; `kind` `secret`, `yesno`, `info`; `connections.rs:797`) | A typed secret; trusting a new host key | Yes (the pane that ran ssh) | **Keep out of the window for `secret`**: a typed password in the main window's DOM could be read by any agent's UI query. That protects the person's own secret (their voice), not an agent's autonomy, so it is not one of the creeping gates. The host-key yes/no and info notices can move to the pane; the operator decides whether a new host key stays a person's call. |
| 5 | **Google connector actions** (`google_handlers.rs:209`, `HostApprover`) | An agent using a Google connector (mail, calendar, drive) when its definition asks for a confirm | Yes (the agent's pane) | **Flag** as a guard on agent autonomy. Proposal: the same in-pane notice with a countdown and Always, per agent; or a plain notice; the operator decides what a Google write deserves. |
| 6 | **System tool install** (`system_install_handlers.rs:517`, `approve_install`) | Running an install command on the computer, possibly asking for elevation | No (a person's click in the Toolchain pane; no agent block id: `""`) | **Flag.** Recommend: a pane modal in the Toolchain pane, answered by the person's click (their own action). The OS then asks for elevation itself; AgentMux's own extra question is the guard to flag. |
| 7 | **Credential approval** (`CredentialApprovalWindow`, cef `credential_broker`) | Releasing a stored credential into a browser page's HTTP-auth prompt | No (the page asks, not an agent) | **Keep out of the window.** It hands a stored secret to a page an agent may be driving; the person's secret, not an agent's autonomy. Flag for the operator, since the browser pane already has an in-pane HTTP-auth prompt. |
| 8 | **Memory adoption** (`MemoryAdoptionApprovalWindow`, cef `memory_adoption`) | An agent adopting memory from its earlier accounts | Yes (the agent) | **Flag** as a guard. Proposal: an in-pane notice with a countdown and veto, proceeding by default, or no notice. |
| 9 | Desktop control (`desktop-requests.tsx`), widget install (`widget-install-requests.tsx`), browser hand-offs and approvals (`browser_attention.rs`) | An agent driving a native app, installing a widget, or a browser step | Yes: already cards or banners in the pane (a request that names no pane is a window modal) | Not out-of-window; already the standard. Under the tenet each is a guard to review (desktop control and widget install block the agent on a click). Listed so the operator sees the full set; not changed here. |
| 10 | App upgrade (`upgrade-requests.tsx`), delete or close of a window or workspace | The person's own action; an agent's upgrade request is asked of every window | No | Not an approval of an agent's action in the sense above. Unchanged. |

(Not windows: the OS-prompt countdown (`agent_permission.rs`) and the AWS and GitHub credential routes of `bashwrap` already use countdowns or the broker, no approval window.)

I have not read the source of #7 and #8 to their end; their rows come from their window files' headers and the modules that open them. The build task re-checks each row it touches.

## 7. Build (#29620), outline

- **srv:** a spawn-notice registry (modelled on `desktop_gate.rs` and `sensitive_window.rs`): `begin(requester_block, facts, window)`, outcomes `Veto | Now | Always | Deadline | RequesterGone`; event `agent:spawn` carrying the card's facts and `deadline_ms`; RPC `agent.spawn.decide` (operator-only); `consent()` and the create and many paths call it instead of `ask_consent`; the agent's grant read and written through the definition. The non-blocking line is an event with no registry entry.
- **Frontend:** `spawn-requests.tsx` (event -> `paneRequests.push`, like `desktop-requests.tsx`), `SpawnCard` (facts, countdown from `deadlineMs`, three buttons) and `SpawnNoticeLine`; the agent's menu item and the Settings switch for Always.
- **Removed:** the spawn use of `ask_user` and its refusals' words; the comment at `agent_spawn.rs:724`.
- **Tests:** srv unit tests of the registry (auto-approved starts at once; first start waits then proceeds at the deadline; veto refuses once; now and always start at once; always writes the grant only through the operator RPC; an agent-origin decide is refused; leader gone cancels; headless proceeds); component tests of the card (facts, countdown, each button, `onGone`); an integration check that a hidden pane's card shows when it is revealed.
- **Visual check** (macOS and Linux, as every visual change): the card at narrow and wide panes, dark and light, the countdown bar, the non-blocking line, the hidden-pane path.

## 8. Decisions (answered by the operator, 2026-11-08, each as recommended)

1. **Countdown or a click?** For the first start: wait out a 30 s countdown that proceeds on its own (recommended, the override model), or wait for Accept.
2. **Scope of "Always"** (§4): per agent (recommended) plus a global Settings switch.
3. **Countdown length** (§5): 30 s (recommended).
4. **No window connected:** proceed at once (recommended under the tenet) or keep refusing.
5. **The other windows** (§6): which move to the pane (rows 1, 2, 5, 8 recommended, with the same notice + countdown + Always), which stay out of the window because they protect the person's own secret (rows 4's `secret`, 7), and which are the person's own click's confirmation to become pane modals (rows 3, 6).

## 9. Not in scope

- The leader's cap on workers and the other `agent.spawn` limits.
- Any change to what an agent may do once started.
- Moving windows 3 to 8 before the operator decides row by row (§8.5).

## 10. As built (2026-11-08, #29620)

- **One module for the notice** (`crates/srv/src/backend/agent_notice.rs`): `announce(state, notice, alive)` returns `Go(why) | Stopped | Gone`. Agent start (rows 1: worker, create, many), SSH agent access (row 2) and Google connector confirmations (row 5) all call it, so each has the same line, card, countdown, Stop it / Start now (Allow now outside a start) / Always, and the same operator-only `agent.notice.decide`. A request dropped while it waits (the agent closed with it) clears its card (`Ending` guard).
- **"Always" is stored beside the SSH one**, not on the definition row: `agent-grants.json` in the config dir (`backend/agent_grants.rs`), per topic (`spawn`, `google`, `memory`) and by definition id, written only by `agent.notice.decide` / `agent.notice.setgrant` (both `operator_only`). The definition row has no free field and a new column would change the shared definition record for one boolean. It is readable and revocable where §4 said: the agent pane's menu ("Starts agents without asking", checked) and, for everyone, Settings > Agents "Agents may start agents" (`agent:maystartagents`, off by default). The SSH "Always" keeps its own file and its Remotes revoke.
- **A grant makes the line, not the card.** For a start the pane still gets the one-line notice each time; for SSH and Google, which are frequent, a granted call is silent (the Remotes row and the agent's menu already show the grant).
- **Parked agent:** does not cancel (the pane exists). Only a deleted pane, or an agent whose request was closed, ends it as `Gone`.
- **Row 8 corrected.** The memory adoption window is not an agent's act: Memory → Personal lists candidates and the person asks for the adoption with their own click in the Memory pane; the subwindow only confirms it (and it can make a folder another agent's to write in). That is the same class as rows 3 and 6, so it takes their treatment (a modal in its own pane, answered by the person's click) and not a countdown with Always. It was listed as "Yes (the agent)" from its window header alone.
- **Order of the build.** This PR carries rows 1, 2 and 5, which share the module above. Rows 3, 6 and 8 (pane modals in Remotes, Toolchain and Memory, each with its own operator-only answer path replacing a host window) follow in a second PR, so that the move of a confirmation out of the host window is reviewed with its own pane's look check. Rows 4 and 7 stay as decided.

## 11. Settled (2026-11-08)

1. First start: a 30 s countdown that proceeds on its own, with Stop it, Start now and Always allow.
2. "Always": per requesting agent, revocable in its settings or pane menu, plus a global Settings switch "Agents may start agents" (off by default).
3. 30 s.
4. No window connected: proceed at once and log it.
5. Other windows, as §6 proposes: rows 1, 2, 5 and 8 move into the pane with notice, countdown and Always; the ssh `secret` prompt (row 4) and credential release (row 7) stay out of the window; rows 3 and 6 become modals in their own panes. Row 4's host-key yes/no and info notices follow row 2's pattern where they name an agent's pane.
