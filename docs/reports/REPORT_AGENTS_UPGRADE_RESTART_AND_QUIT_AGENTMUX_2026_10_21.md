# Report: agents upgrade, restart and quit AgentMux themselves, behind a 10-second red alert countdown

**Status:** analysis. Written 2026-10-21 against `main` at 379e7d5a0. It records the operator's decision and the design it implies. It changes no code. §8 lists the specs to update when it is built.
**Author:** Manoz@Area54, at the operator's request
**Trigger (operator, 2026-10-21):** after an agent-started upgrade stopped at the launcher's confirmation dialog: "agentmux are first class with autonomy, so restarting themselves is part of what is needed from them. but the spirit of the warning is good. We dont want the system prompt, instead, we want a 10 second read alert countdown text. This also applies to restarting an instance of agentmux. Lets formalize this solid into a report. Agents would be able to both upgrade, restart, and even terminate agentmux"
**Related:** `SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` (the upgrade flow and the `App` tool), `SPEC_AGENT_SELF_QUIT_2026_09_24.md` §6.3 and §6.5 (the provenance gate and the 15-second override this generalises), `SPEC_SHUTDOWN_COUNTDOWN_MODAL_2026_09_04.md` (a launcher-owned countdown for quitting, proposed), `crates/launcher/src/upgrade_restart.rs` (the dialog this removes).

## 1. The decision

An agent may **upgrade**, **restart** and **quit** AgentMux on its own judgment. Nobody grants permission first. Instead, every window shows a **red alert with a 10-second countdown**, saying who is doing what and why. The user can cancel during those 10 seconds. If they don't, it happens.

The model changes from "ask, then act" to "announce, then act unless stopped". AgentMux already decided this for one agent shutting down another (`SPEC_AGENT_SELF_QUIT_2026_09_24.md` §0.1-3: "an opt-out countdown, not an opt-in dialog"). This report applies the same decision to the whole app.

## 2. What happens today

On 2026-10-21 the operator told an agent: "jekt all agents on this machine, warn them, and then do the upgrade". It took three steps and still needed the operator:

1. The agent messaged every agent on the machine by hand to warn them.
2. It called `App` `upgrade` with the operator's words as `user_instruction`. srv's provenance gate (`sagas::self_quit::gate`, called from `app_api::app_update::handle_request_upgrade`) accepted them, because the operator's own message started the turn.
3. The launcher then stopped and asked again, in a native Windows dialog (`upgrade_restart.rs` `confirm`, a `MessageBoxW`). The code says it was built so that "no page or agent tool in AgentMux can click" it. The agent could do nothing more, and the operator had to click.

What an agent can do today:

| | Upgrade | Restart (same version) | Quit AgentMux |
|---|---|---|---|
| On the user's words this turn | Yes, then the launcher's dialog | No path | No path |
| On its own judgment, or from a message | `pending_user`: an Upgrade now / Later prompt in every window, then the launcher's dialog | No path | No path |
| The user | Panel button, then the dialog | No button (quit and start again) | Tray or menu Quit (`quit_app`, `crates/cef/src/commands/window/lifecycle.rs`) |

So an agent can never finish an upgrade alone, and can't restart or quit AgentMux at all. Today's safeguards answer "is this allowed?" with two prompts, which the operator now finds the wrong question for agents that are meant to run on their own.

## 3. What stays from today's design

The warning is kept. The operator: "the spirit of the warning is good".

- **The user always gets notice before AgentMux restarts or quits.** An agent can still be wrong, or be talked into it by something it read (`SPEC_AGENT_SELF_QUIT_2026_09_24.md` §6.3's residual risk). The countdown is how the user catches that.
- **Wait for other agents' turns.** `update_restart.rs` already waits up to 10 minutes (`WAIT_FOR_AGENTS`) for working agents to finish before restarting. That stays, and it is the right time to warn them (§4.4), which the agent did by hand on 2026-10-21.
- **Only the release AgentMux found itself is installed, verified against its checksum.** Autonomy doesn't extend to choosing a file or a URL.
- **Every request is audited:** who asked, why, what started their turn, and what happened.
- **Each agent resumes afterwards and is told** (`backend/upgrade_notice.rs`), now including who restarted AgentMux and why.

## 4. Design

### 4.1 The three actions

On the `App` tool, next to `version`:

```
App({ action: "upgrade", reason, version? })   → scheduled | up_to_date | unavailable
App({ action: "restart", reason })             → scheduled
App({ action: "quit",    reason })             → scheduled
```

- `user_instruction` is no longer needed. If given, it is recorded and shown in the alert ("at the user's request"), but nothing depends on it.
- `upgrade` keeps everything §3 lists: the latest release only, the checksum, downloading first.
- `restart` relaunches the same version: the launcher quits the host as the tray's Quit does (windows close and the session is saved), waits for srv to stop, then starts itself again. The upgrade path already does all of this except installing.
- `quit` stops AgentMux and doesn't start it again. Every agent in it stops, including the caller. The caller is told so in the tool's description and in its answer.

Each answers at once with `scheduled`, the request id and the earliest time it can happen. The agent ends its turn. The outcome reaches it as an `[AgentMux]` note (`cancelled by the user`, or, after a restart, the resume note), or not at all, for `quit`.

### 4.2 The sequence

1. **Request.** srv records it: who, `reason`, the turn's provenance, and the action.
2. **Prepare.** For `upgrade`, download and verify if needed. The alert doesn't show yet: the user would have nothing to decide while a download runs.
3. **Warn the agents** (§4.4) and **wait for working agents to finish their turn**, up to `WAIT_FOR_AGENTS` (10 minutes). The alert doesn't show yet either.
4. **Countdown.** The red alert shows in every window for 10 seconds (§4.3), with the tone and an OS notification.
5. **Act.** The launcher restarts, upgrades or quits. No dialog.

The countdown is the last step before acting, so the notice comes when the user can still do something, not ten minutes earlier.

### 4.3 The red alert

- **Where:** across the top of every AgentMux window, above everything, in the theme's error colour (`am-error`). One line, live:
  **"Lazo is restarting AgentMux in 10 s: the CEF upgrade needs a fresh host. [Cancel]"**
  For an upgrade: "… is upgrading AgentMux to 0.59.37 in 10 s …". For quit: "… is quitting AgentMux in 10 s. Nothing will start it again …".
- **Who and why:** the requesting agent's name, and its `reason`, cut to one line, with the full text in the tooltip. A second line names the agents that are still mid-turn and will be interrupted, if any.
- **Cancel:** one button. The answer reaches srv only through the host (`approvals.*` → `/api/v1/host/…`, checked with the host's token), as approvals do today. So an agent can't cancel the countdown for the user, and can't take away the user's chance to cancel.
- **No "Now" button.** It would have to be another click, and the countdown proceeds by itself anyway.
- **Sound:** the shutdown-pending tone (a falling minor arpeggio, `SPEC_AGENT_SELF_QUIT_2026_09_24.md` §6.5), once at the start and once at 3 s. It has its own setting under the sound master switch.
- **OS notification:** through the notification router, Attention priority. A user in another app still hears about it. Clicking it focuses the alert.
- **No window open** (tray or background mode): the notification and the tone only. If nobody cancels, it proceeds, as §6.5 decided for agent shutdowns: the countdown protects a user who is there, and doesn't hold AgentMux for one who isn't.
- **A frozen frontend:** restarting is the cure for a frozen host, but a frozen renderer can't draw the alert. Then the launcher shows the countdown in its own native window, as the startup splash does (`crates/launcher/src/splash.rs`). This is the window `SPEC_SHUTDOWN_COUNTDOWN_MODAL_2026_09_04.md` proposes for Quit.

### 4.4 Warning the agents

When a request is accepted (step 3), srv sends every agent in the instance one `[AgentMux]` note:

`[AgentMux] Lazo is restarting AgentMux (the CEF upgrade needs a fresh host). It waits for your current turn to end, then counts down 10 s. Commit what you'd hate to lose and don't start long builds or tests. Your session resumes afterwards; background processes you started will be stopped.`

This automates what the operator asked an agent to do by hand. The existing "what the restart stops" list (`update_restart::stopping`) goes in each agent's note when it names that agent's background processes.

### 4.5 Restarting or quitting another instance

"This also applies to restarting an instance of agentmux": an agent can restart or quit another AgentMux instance on the same machine, such as a dev build it started or another channel. The cross-channel path that `SPEC_AGENT_SELF_QUIT_2026_09_24.md` §6.5 uses for stopping an agent applies:

- The caller sends the request to that instance's srv, with that instance's `auth_key` from the shared registry.
- That instance runs the sequence, and its own windows show the countdown. The user sees the alert on the instance that is going away.
- Outcomes are proxied back, so the tool waits the same way.

An instance older than the route refuses with 404, and the tool says so. It doesn't fall back to killing processes.

### 4.6 Repeats and loops

Autonomy needs a brake, because a confused agent could keep restarting, or restart every time it resumes:

- **One request at a time.** A request while one is scheduled joins it and gets the same id and deadline. Requests are ranked quit > upgrade > restart, and the higher one replaces a lower one, with a fresh countdown.
- **A rate limit.** Once AgentMux has been restarted by an agent, another restart request in the next 10 minutes is refused, with the time it can be asked again. The resume note says who restarted it and why, so an agent doesn't restart again for the same reason.
- **A cancel holds.** After the user cancels, the same agent's same action is refused for 30 minutes, unless the user asks for it in a turn of their own. A cancelled countdown must not come straight back.
- **A crash loop doesn't count.** A restart caused by a crash, rather than a request, doesn't go through this path, and doesn't count against the limit.

### 4.7 What the user's own words change

Under this model, a user who asks for an upgrade sees the same 10-second alert. The alert says so ("at your request"), and nothing else changes. One path is simpler to trust than two. The countdown also tells the user's other agents what's about to happen, even when the user's own words started it.

## 5. Risks, and why they're acceptable

| Risk | Bound |
|---|---|
| A prompt-injected agent restarts AgentMux | 10-second alert in every window, tone and notification; one restart per 10 minutes; sessions resume; audited with provenance. |
| A prompt-injected agent quits AgentMux | The same alert, which says nothing will start it again. Nothing is deleted: the user starts AgentMux and everything resumes. This is the worst outcome, and it is a stop, not a loss. |
| The user is away for the 10 seconds | It proceeds. A restart resumes everything. A quit leaves AgentMux closed until the user returns. Revisit with the audit log if quits by agents turn out to be a problem (open question 2). |
| Agents interrupted mid-turn | Only after waiting up to 10 minutes for their turn to end, and they are warned at the start of the wait. |
| A remote agent (LAN or WAN) asks a local agent to restart | The local agent decides, as it decides anything else, and the alert names the local agent. A message can't call `App` directly. |

## 6. Phases

1. **Restart and the countdown.**
   - The `restart` action.
   - The red alert, the tone and the host-only Cancel.
   - The agents' warning note, the rate limit and the audit.
   - The resume note names who restarted AgentMux and why.
2. **Upgrade without a dialog.**
   - Remove `confirm` from `upgrade_restart.rs` and the `pending_user` prompt (`upgrade-requests.tsx`), and route `upgrade` through phase 1's countdown.
   - The panel's own Upgrade & restart button uses the same alert, so there is one path.
3. **`quit`.** The action, the stronger wording, and the 30-minute hold after a cancel.
4. **Other instances** (§4.5), and the launcher-native countdown for a frozen frontend (§4.3).

## 7. Open questions

1. **10 seconds here, 15 seconds for stopping one agent** (`pending_shutdown::OVERRIDE_WINDOW`). Should both be 10?
2. **Quit while nobody is there.** Should a `quit` with no window open wait for the user to come back, or proceed like a restart? This report proposes proceeding, to match §6.5.
3. **Should a restart the user asked for skip the countdown?** §4.7 proposes no.
4. **The provenance gate.** `sagas::self_quit::gate` stays for `QuitSelf` (one agent quitting itself). Should `QuitSelf` move to the same "announce, then act" model, for one rule everywhere?

## 8. Specs to update when this is built

- `SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md`:
  - §5.1: `upgrade` "never restarts on the agent's word alone" becomes the countdown.
  - §8 decision 3.
  - Phase 3b's launcher dialog.
- `SPEC_AGENT_SELF_QUIT_2026_09_24.md`: a pointer from §6.5 to this, and a decision on open question 4.
- `SPEC_SHUTDOWN_COUNTDOWN_MODAL_2026_09_04.md`: its launcher window becomes the fallback for §4.3.
- The MuxAPI operator-config entry (`crates/srv/operator-config-seed.json`): the `App` bullet says an upgrade "never happens on your word alone". It needs a version bump so installs pick up the new text.
- `crates/mcp/src/tools/app_tool.rs` and the tool schema descriptions.
