# Spec: agents put Kanban tasks on hold and take them off hold

**Status:** active — implemented in #5326.
**Date:** 2026-10-22
**Author:** Naki-2@narko, at the operator's request
**Verified against:** `agentmux` `main` @ `31bf85d31` (2026-10-22), by reading code.
**Changes:** `SPEC_WARDEN_KANBAN_2026_10_20.md` §6.4 (who moves what: hold and unhold), §6.12 (the `Tasks` actions), §6.13 (trust: operator-only moves).

## 1. Ask

The operator, 2026-10-22, on finding two finished tasks they had to take off hold by hand before the agent could hand them in: "why were they put on hold? agents are first class, they can set and take off holds at will. if the code doesnt support it, write a spec to file".

## 2. Today

**Why the two tasks were on hold.** The agent filed them for itself (`enqueue` with `for: "me"` and `hold: true`). Each one was waiting on something outside its control: a reply from a third party, and the operator's yes on three items. The hold kept them out of `claim` and out of the backlog reminder until then. Both conditions were met the same day and the work was done. But the agent couldn't take its own hold off, so it couldn't claim the tasks or hand them in. Both waited for the operator to click, only to lift a hold the agent itself had set.

**What the code allows.**
- **Setting a hold:** an agent can put a task on hold only when it creates it. `Tasks` `enqueue` takes `hold` (`crates/mcp/src/tool_schemas.rs`: "on hold until the operator releases it"), and `POST /agentmux/work` files the task as `held` (`server/work_queue.rs`).
- **Releasing a hold:** only the operator can, from the Kanban (`kanban.act` `move` with `hold: false`, `server/app_api/kanban.rs`). That module is operator-only by construction: it is a websocket RPC from the app's window, never reached from an agent's MCP.
- **An existing task:** no agent route puts it on hold or takes it off. The routes are `enqueue`, `list`, `claim`, `heartbeat`, `complete`, `release`, `update`, `ask` and `submit`, plus `cancel`, which refuses agents (`server/routes.rs`).
- **The storage already does it:** `work_hold` (open → held), `work_unhold` (held → open) and `work_start_for` (claim one named waiting task, open or held) in `backend/storage/work_board.rs`. History rows `held` and `unheld` exist (`db_work_events`).
- **`claim` takes the next eligible task:** the caller's own tasks first, then priority, then age. It can't name a task. So after a hold is lifted, an agent with other tasks waiting may be handed a different one.

**Where this is written as a rule.** `SPEC_WARDEN_KANBAN_2026_10_20.md` §6.4: "within Backlog: reorder, move to another agent or to Anyone, hold, unhold | operator". §6.13 lists "moves from Backlog" among the operator-only moves.

## 3. Design

### 3.1 Two new `Tasks` actions: `hold` and `unhold`

| Action | Input | Effect |
|---|---|---|
| `hold` | `id`, `reason?` | A waiting task (`open`) becomes `held`: no `claim` takes it and no reminder or finished-task answer names it. Its own claimed task: see §3.2. |
| `unhold` | `id`, `reason?` | A `held` task becomes `open` again, for whoever it was for. |

- **Routes:** `POST /agentmux/work/:id/hold` and `POST /agentmux/work/:id/unhold` in `server/work_tasks.rs`, beside `update`, `ask` and `submit`. Same auth, same `Caller` and actor check as the other agent routes. Each accepts a task number or id (`resolve_id`).
- **Who may:** any agent, on any waiting task. That includes tasks for another agent or for anyone, and holds set by the operator or by another agent. The operator asked for this: agents set and take off holds at will.
- **History:**
  - Each move writes a `held` or `unheld` row with the agent's name, actor kind `agent`, and the reason as its body.
  - The card's drawer history shows who held it and who lifted it, with the reason ("Naki-2 put it on hold: waiting on GitHub Support", "Naki-2 released it from hold"); it already renders an agent actor by name.
  - `publish_changed` refreshes the board.
- **Answers:**
  - Success says the new state: "#110 is on hold" / "#110 is off hold and waiting for Naki-2".
  - A task in any other state is refused with its state ("#110 is in review; only a waiting task can be put on hold").
  - A task already in the asked-for state is not an error: the answer just says so, and no history row is written.
- **Reason:** optional, at most `MAX_TEXT`. It goes through the same sensitive-text check as notes (§6.13 of the Kanban spec).

### 3.2 Holding a task the agent is working on

An agent holding a claimed task may put it on hold with its `attempt`. This is "I can't go on until X happens".
- The claim is given up and the task becomes `held`. It stays the agent's own: a task it took from anyone becomes its own, so the pause comes back to it (unlike `release`, which hands such a task back to anyone).
- The attempt isn't counted against the task, because pausing is not failing: the store raises `max_attempts` by one in the same update. It never lowers `attempts`, which is the claim's fence: the next claim gets a new number, so a late call from the paused claim is refused (Codex on #5326).
- The card leaves In progress for the Backlog, greyed "on hold", with the reason.

Any other agent's `hold` on a claimed task is refused ("#12 is being worked on by Loap-3; message them"). A task someone is working on is that agent's to pause.

For a `blocked` task (a question waiting on the operator), `hold` is refused. The question is already the pause, and the operator's answer resumes it.

### 3.3 `claim` can name a task

`claim` gains `id`. With it, srv claims that task, and only that one, if:
- **It is waiting:** `open`, or `held`. Claiming a held task lifts the hold; the history records `unheld` and then `claimed`, both by the agent.
- **The caller may take it:** it is for the caller, or for anyone, or for a group the caller is in, and of the `kind` the claim names, if it names one. These are the same eligibility rules as an ordinary claim, by UID first (`ClaimFilter`).
- **It isn't waiting on unfinished tasks** (`after`, §6.15). Otherwise it is refused, naming the unfinished task. The operator's **Start** still overrides the wait; an agent's claim doesn't.

A new store function, `work_claim_one(id, filter, now, lease)`, does it in one `UPDATE … RETURNING`, like `work_start_for` but with the eligibility and dependency checks in the `WHERE` clause. That way a task that moved in the meantime is never taken. It reaps first, like `handle_work_claim`. The answer is the same as an ordinary claim (`claimed`, `attempt`, `num`, `item`), or `{claimed: false, why}`.

Without `id`, `claim` is unchanged.

So the case in §2 becomes three calls, with no operator click: `unhold` (or straight to `claim` with `id`, which lifts the hold), then `submit`.

### 3.4 What stays the operator's

Accept, Cancel, Send back, Answer, Start on…, and Retry stay operator-only: they are reviews and decisions. The same goes for reordering a backlog and giving a waiting task to another agent or to Anyone. This spec changes holds only.

The operator's ways to stop an agent are kept, and an agent can't undo them:
- `kanban:continue: false` on its block, which stops finishing a task from naming the next one and stops the reminder;
- Cancel;
- moving its tasks to someone else.

The Kanban spec's §6.5 "Stopping it" names putting tasks on hold as one of these. That no longer holds once an agent can lift the hold (D1).

### 3.5 Wording

- **`enqueue`'s `hold`:** "on hold: no one takes it until it is taken off hold (`unhold`, by you, another agent or the operator)".
- **The `Tasks` tool's description** gains: "hold / unhold: put a waiting task on hold, or take it off. Use hold when a task waits on something outside your control (a reply, a decision), and unhold it yourself when that arrives. claim with id takes that task."
- **The operator-config guidance** (`crates/srv/operator-config-seed.json`, the Tasks paragraph) gains one sentence to the same effect.
- **Kanban spec:**
  - §6.4's row becomes "hold, unhold | operator or agent".
  - §6.12 gains the two actions and `claim`'s `id`.
  - §6.13 drops holds from the operator-only list.
  - §6.5 "Stopping it" points at §3.4 here.

### 3.6 Older builds

The queue is shared by every AgentMux on the computer (Kanban spec §10). An older srv has no `hold` or `unhold` route, and its claim ignores `id`.
- The MCP and srv ship together, so an agent only meets this when its own srv is older than its MCP, which doesn't happen in a normal install.
- Should it happen, `hold` and `unhold` answer 404, and the tool says "this AgentMux doesn't support hold yet".
- An old srv's `claim` with an unknown `id` field takes the next task as before. The new MCP checks the returned task's id and, if it isn't the one asked for, releases it at once (a release, so it uses one of that task's attempts) and says so.

## 4. Files

- `crates/srv/src/server/work_tasks.rs`: `handle_work_hold`, `handle_work_unhold`; holding one's own claimed task (§3.2).
- `crates/srv/src/server/routes.rs`: the two routes.
- `crates/srv/src/server/work_queue.rs`: `ClaimRequest.id`; `handle_work_claim` calls `work_claim_one` when it is set.
- `crates/srv/src/backend/storage/work_board.rs`: `work_claim_one`; `work_hold_own` (claimed → held for its holder, fenced; its attempt limit grows by one).
- `crates/mcp/src/tool_schemas.rs`, `crates/mcp/src/tools/work.rs`: the actions, `claim`'s `id`, the wording (§3.5).
- `crates/srv/operator-config-seed.json`: the guidance sentence.
- `frontend/app/view/warden-kanban/`: no change; the drawer's history already renders `held` and `unheld` rows with an agent actor by name.
- `docs/specs/SPEC_WARDEN_KANBAN_2026_10_20.md`: §6.4, §6.5, §6.12, §6.13 (§3.5).
- agentmux-docs: the MuxAPI page is generated from `tool_schemas.rs`; the Kanban page's "who moves what" gets the same change.

## 5. Tests

- **Store:**
  - hold moves `open` → `held` only;
  - unhold moves `held` → `open` only;
  - holding its own claimed task costs no attempt, keeps the fence counting (a late call from the paused claim is refused) and keeps the target;
  - a stale `attempt` is refused;
  - `work_claim_one` takes an eligible open or held task, refuses one for another agent, one waiting on an unfinished task, and one that moved meanwhile.
- **Routes:**
  - an agent's hold and unhold succeed on its own task, another agent's waiting task and an operator-held task;
  - they are refused on `claimed` (not its own), `blocked`, `review`, `done`, `failed` and `cancelled` tasks;
  - each writes one history row with the agent as actor and the reason;
  - a repeat is a no-op without a row.
- **Claim with `id`:** a held task is claimed with `unheld` and `claimed` rows; without `id`, behaviour is unchanged (existing tests).
- **MCP:** the schema lists the actions; `work.rs` maps them to the routes and words the answers.
- **Reminder:** a held task is still never named, whoever held it.

## 6. Decisions for the operator

- **D1. Can an agent lift a hold the operator set?**
  - *As asked:* yes, any hold. The card and its history show who set it and who lifted it, and the operator's stronger stops (§3.4) stay theirs.
  - *Alternative:* an operator-set hold can only be lifted by the operator; agents can lift their own and each other's. That keeps "put it on hold" as a stop the operator can rely on, at the cost of the case in §2 whenever the operator set the hold.
  - *Recommended:* as asked.
- **D2. Can an agent hold a task that is for another agent?**
  - *As asked:* yes, on any waiting task.
  - *Alternative:* only its own tasks and Anyone's.
  - *Recommended:* yes, as asked. The history names who did it, and the other agent can lift it.
- **D3. Pausing one's own claimed task (§3.2):** included, costing no attempt. *Recommended*, since that is the natural way to say "blocked on something outside".
