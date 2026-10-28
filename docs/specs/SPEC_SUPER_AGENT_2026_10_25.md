# Spec: the super agent, one place for sensitive operations

**Status:** active — approved by the operator, 2026-10-25 ("looks good", relayed by AgentY), with the answers to the open questions in section 10. Built in steps (section 9); steps 1–5 are built (#5634 for the `super` seat, #5654 for the person's RPC, the banner in this one).
**Date:** 2026-10-25
**Author:** Parlo@charlie
**Changes:** who may trigger the sensitive operations in section 2, and how the Keep running window (`sagas/pending_shutdown.rs`) generalises to them.

## 1. Why

The operator's rule: agents can do anything, except tell another agent (or a record) that the operator said something they didn't. A few operations are too sweeping to leave to any one agent's say-so: they end or reshape everyone's work. Those go to one designated agent, the super agent, with a visible countdown and a veto for the human. This is routing and oversight, not a ban: the agent that asks is not the one that decides, and the person can always stop it in time.

The model already exists: `Stop agents` and `ClosePane` open the 15 s keep-running window (`sagas/pending_shutdown.rs`: banner on the pane, chime, OS toast, Keep running; at the deadline the action runs). The super agent generalises that window to the other sensitive operations and puts one agent's review in front of it.

## 2. Which operations

| Operation (`op`) | Today | Countdown | What the banner shows |
|---|---|---|---|
| `stop_agents`, `close_pane` | 15 s window | 15 s (unchanged, no super agent in front: the window is the oversight) | asker, target |
| `quit_agentmux` (close this AgentMux) | the window | 2 min (a restart of AgentMux is critical) | requester, super agent |
| `upgrade` | operator's words gate | 2 min (a restart of AgentMux is critical) | requester, target version |
| `widget_install` | the person's prompt | 30 s | widget, publisher, every permission |
| `desktop_control_grant` | consent dialog | 30 s | requester, scope |
| `vault_use` | per-use approval | 30 s per use | requester, entry label and kind (never a value) |
| `vault_always_allow` | the person's | 30 s | requester, entry label, what it permits |
| `sender_key_repin` | the person's | 30 s | agent name, old and new fingerprint |
| `job_template_install`, `job_template_approve` | the person's | 30 s | template, publisher |
| `tower_pair` | the person's | 30 s | device |

The operator's windows: 30 s by default, at most 2 min, and only for the critical operations. Kept in one table in srv (`backend/sensitive_ops.rs`), so adding an operation is one row: its countdown, whether the super agent reviews it, and the facts its banner shows. The banner text is built by AgentMux from the structured request, never from the requester's words (as `toast_body` and `clean_by` do today).

## 3. The super agent

- A seat role, `super`, set like any seat (`agent.setSeat`, and `Agents set_seat`). `ROLES` in `backend/agent_seat.rs` gains it. At most one in the shared agent roster (a seat is stored on the shared definition, so every AgentMux on the same data home sees the same holder): setting it on a second agent moves it, and the first reverts to the role it had (the seat remembers it as `prior_role`; worker when it had none), keeping its qualifier, lead and area. Naming two super agents in one call is refused. An AgentMux that doesn't share that roster has its own holder. Only a holder that is running in the AgentMux where a request is made can approve it (section 4).
- It is an ordinary agent: it reads requests, may check facts (look at the widget's folder, read the Vault entry's label, run `App version`), and approves or denies with a reason. It holds no extra tools. What makes it special is that srv accepts approvals for sensitive ops only from the agent whose seat is `super`.
- The super agent may itself ask for a sensitive op. It approves its own request; the countdown and veto are the oversight, so nothing is gained by blocking self-approval.
- It is recorded as the approver, never as the operator. Its approval is not "the operator said yes".

## 4. How an agent asks

One MCP tool, `Sensitive`, and one HTTP route family under `/api/v1/sensitive/` guarded by `agent_only` (agent token required, requester proven by `proven_slug`, as `agent_routes` does):

- `request {op, params, reason}`: srv records a request (id, requester uid and slug, op, params, reason cleaned by `clean_reason`, status `asked`). It sends the super agent one message: "<requester> asks for <op>: <structured summary built by srv>. Approve or deny with Sensitive." The message is from the system, labelled as the requester's request; nothing in it is the operator's voice. Returns the request id and status at once.
- `approve {id}` / `deny {id, reason}`: only the agent whose seat is `super` and that is running in this AgentMux. `approve` moves the request to `pending` and opens the countdown (section 5). `deny` ends it and tells the requester.
- `status {id}` / `list`: any registered agent; the requester sees its own, the super agent sees all.
- `cancel {id}`: the requester withdraws its own request while it is `asked` or `pending`.

The existing tools that perform these operations (`App upgrade`, `Widget install`, `Desktop`, `Vault`, `Stop` of other agents/`quit`) keep working and become thin wrappers: they file the request for the caller and wait for the outcome, so an agent that calls `App upgrade` gets the same flow without learning a new tool. `user_instruction` on them goes away.

## 5. The countdown and the human's override

On `approve`, srv opens a window per op (table in section 2). It is a sibling of `pending_shutdown`'s window, in `sagas/sensitive_window.rs`: that one is tied to an agent pane, while these operations (quit, install, grant) are not. It publishes `sensitive:pending` and `sensitive:cleared` for the banner:

- banner on every visible agent pane and the Warden bar, OS toast with the same wording, chime. It names the requester, the super agent that approved, the op and its structured summary (section 2).
- The human can Veto (a button like Keep running, worded for these ops) or Approve now to skip the countdown. No click is required to let it proceed.
- At the deadline the operation runs as the super agent's approved act on behalf of the requester. Each action's own code path runs unchanged (the install, the upgrade, the grant), so no new power is added; only who may trigger it changes.
- A second request for the same op and target while one is pending joins it (the clock never restarts), as `pending_shutdown::request` does for its window.

## 6. No super agent running

The request still gets its window; only the agent in front is missing. If no agent has the `super` seat, or it is not running or does not answer within 2 minutes:

- the request goes to the countdown anyway, with double the op's window up to the 2 min longest (a restart is already at it), the banner saying "no super agent reviewed this";
- the person's veto stays the only check;
- a destructive pair (`quit_agentmux`, `upgrade`) proceeds after the doubled window (up to 2 min) as well; refusing until a super agent exists would be a ban (section 10, item 1).

A super agent that stops while a request is `asked` leaves the request to expire into the same fallback; one that stops while it is `pending` changes nothing (the countdown runs).

## 7. Audit trail

One audit row per step, through `log_fleet_action_audit` (shown in Warden's Audit tab): `sensitive.request`, `sensitive.approve`, `sensitive.deny`, `sensitive.veto` (the person), `sensitive.cancel`, `sensitive.run` (outcome: done, failed, superseded), `sensitive.fallback` (no super agent). Each row has requester slug and uid, approver slug (for approve/deny/run), op, the structured summary, the window used and the outcome. The actor is always the agent that did the step; a veto is recorded as the person's (the window, proven). Request text the requester wrote is stored as written, shown cleaned.

## 8. Forging the operator's voice stays closed

- Nothing in a request, banner or message is presented as the operator's words. The system message to the super agent names the requester and says so.
- A veto or approve-now counts only from the window's proven connection (`operator_only`), never from an agent. This is the one place `operator_only` stays: the human's side of the override.
- `fleet.broadcast` stays window-only, transcripts stay writable only by their own agent, signing keys stay with the window and the agent they belong to.

## 9. Build plan

1. Seat role `super` and the at-most-one rule (built).
2. `sensitive_ops.rs` (table) and the `sensitive` request registry (built).
3. The countdown (`sagas/sensitive_window.rs`): the window, the person's veto and approve-now, the runner each operation registers, the two events, the audit rows for the veto and the run (built).
4. The person's RPC, `sensitive.decide` (veto or approve-now), accepted only from the proven AgentMux window and never from an agent (built).
5. The banner and the chime in the agent pane (built).
6. The OS toast (it shares the notify router with the stop and close toasts, whose wording rules it follows).
7. The routes and the tool, then the wrappers:
   a. HTTP routes under `/api/v1/sensitive/` and the `Sensitive` MCP tool: request, approve and deny (the super agent only), cancel, status, list (built).
   b. Wrap `App upgrade`, `Stop` quit, `Widget install`, `Desktop`, `Vault` to file requests, each registering its runner. They go live after step 8, so a request with no super agent has its fallback.
8. Fallback timer (no super agent) and the audit rows for the request, approval and denial.
9. Remove the `user_instruction` gates they replace.

One PR per step, each behaviour-only text.

## 10. Decisions on the open questions

1. No super agent, destructive ops: proceed after the doubled window (up to 2 min). Refusing would be a ban.
2. Vault `use`: a per-session grant after the first countdown for each agent; every use is still in the audit log.
3. A host with no super agent defaults to its first leader seat until a seat names one.
4. A leader stopping its own spawned worker skips the window; it is logged.
5. A request by the super agent itself gets the same countdown, self-approved and logged.
