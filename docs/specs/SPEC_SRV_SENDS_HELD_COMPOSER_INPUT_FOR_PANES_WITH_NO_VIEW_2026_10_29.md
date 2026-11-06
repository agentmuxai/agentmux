# Spec: srv sends the operator's held composer input when the agent has no pane

**Status:** draft — for the operator's decision on §6. Kanban #7464 (narko); from Naki-2's recommendation in #5708.
**Date:** 2026-10-29
**Author:** Manoz@Area54
**Verified against:** `agentmux` `main` @ `58d7cc7b2` (2026-10-29), by reading code. Nothing here is built.
**Builds on:** `SPEC_OFFLINE_RESILIENCE_2026_10_20.md` §4.4 (held input, #5246), `SPEC_ALWAYS_AVAILABLE_AGENTS_2026_10_27.md` (agents that run with no pane).
**Not the same as:** `SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md`. Those are messages *from other agents*, held by srv. This is the *operator's* typed message, held by the pane.

## 1. The problem

When the model API is unreachable (or a turn is retrying), the pane does not send what the operator types: it keeps it under the composer as a held message and sends it by itself, in order and once, when the hold lifts (`offline/offline-input.ts`). The held messages live in the block's meta (`agent:held_input`, `{ agentId, messages: HeldInput[] }`), so a reload or restart keeps them. But the only thing that sends them is the pane's own code (`useOfflineInput.ts`: "a held message goes out only while its pane is open").

With `agents:always_available` on (the default), closing a pane parks the agent: the agent keeps running, its block lives on in no tab, **no view is mounted**, and nothing sends the held message. The operator typed it, saw it held, closed the pane, and it waits until they open that agent again, however long the API has been back. Agents that were never given a pane (a started agent, a restart-resumed one) can hold input the same way if the operator typed into a pane that was then closed.

## 2. What exists

| Piece | Where | Note |
|---|---|---|
| The held list | block meta `agent:held_input`, written by the pane through `setBlockMeta` | Shared by every mount of the pane; each `HeldInput` has `id`, `text`, `heldAt`, `attachments?`, `unconfirmed?`, `attempt?`. |
| The hold rule | `holdsInput(retrying, api)`: the turn is retrying, or the provider's API is `offline`, or `unknown` for under 20 s | `api` is srv's own connectivity state (`connectivity::status()`, service `model_api:<vendor>`), which the pane only reads. |
| The send | the pane's send path, wrapped by `confirmedSend`: accepted only when srv accepts the message (`pending-accepted`) | One at a time, while the pane is idle; a send whose answer never came is `unconfirmed`, kept, and left to the operator. |
| Delivery to an agent with no pane | `COMMAND_AGENT_INPUT` (`agentinput`) and the reactive handler work for a block with a controller and no view | A parked agent already receives messages from other agents this way. |

srv already has the connectivity state the hold rule reads. "The turn is retrying" reaches the pane as srv's `agent-api-retry` event, which the CLI's own frames cause; whether srv keeps that fact where a sweep can read it, and not only as an event, is to be checked when this is built.

## 3. Proposal

srv sends a held message for a block **that no window has mounted**, with an **attempt claim** in the held message itself, so a pane mounting in the middle of a send cannot send it again.

### 3.1 When srv sends

A sweep (woken by the connectivity change and the agent's state change, `held_jekt_wake`'s sibling, plus a slow timer) takes each block whose meta has held input and that has **no pane**: it is parked (`agent:parked`, in no tab), the same test `is_parkable` and the Agents list use. A block shown in a tab is the pane's to send, even in a hidden tab: a mounted-or-not question srv cannot answer for tabs, so the cut is "parked", which it can.

For such a block, when the hold rule's inputs say go (API reachable, no retry in flight, the agent idle, the account able to run: `availability` is `Idle` or `Running` with `would: deliver`), srv sends the **first** held message, waits for the controller's acceptance, removes it, and goes on to the next, one at a time and in order, as the pane does.

### 3.2 The claim

Per message, in the same meta object: `claim: { by: "srv:<boot_id>", at_ms }`.

1. srv sets the claim on message *m* with a compare-and-set on the meta (the message is still there, unclaimed or its claim is older than 2 min).
2. It sends. On the controller's acceptance it removes *m*; on a refusal it clears the claim and stops the sweep for that block until the next wake; on no answer in 2 min the message stays claimed and the claim ages out into the pane's existing `unconfirmed` state, which asks the operator.
3. A pane that mounts reads the list: a message with a claim younger than 2 min is **in flight**, shown as "sending", not sent by the pane; an older claim is `unconfirmed`. A pane never sends a claimed message; srv never sends a message the pane has marked in flight (`attempt` set).
4. The pane clears nothing it did not set.

This makes the send at-most-once per attempt (the claim is written before the send), and a crash between claim and send leaves the message to the operator rather than sending twice, which is what the pane already does for an unknown fate.

### 3.3 What srv does not do

- A `/` or `!` command is never held, so never sent by srv.
- A message whose send needs the pane (runtime args chosen in the composer, a login guard the pane shows) is **left held** for the pane, with a note on the Swarm row. In the first version srv sends only plain text and attachments by reference.
- It does not open a pane.

### 3.4 Showing it

"N held" on the Swarm row (the card's cheap first step) is **Camper's**: it reads `agent:held_input` and the claim, nothing from srv's sweep.

## 4. Tests

- A parked block with two held messages and the API reachable: both go out in order, the list ends empty.
- The API offline, or a retry in flight, or the agent not idle: nothing goes out.
- A pane mounting while the claim is fresh does not send it; with the claim older than 2 min it shows it as unconfirmed.
- A refusal clears the claim and does not loop; no answer leaves it claimed.
- A block in a tab is never swept.
- A held `/` command, or one that needs runtime args, is left alone.

## 5. Risks

- **Double send across a crash.** Only the claim-before-send order prevents it; a lost claim write means no send. Test the order.
- **The pane's idle test is not srv's.** A pane's `working()` comes from its own state store; srv uses the controller's `turn_active`. They agree for a parked agent (no pane to disagree).
- **A message the operator would have edited.** The pane lets the operator Cancel a held message; a parked agent has no composer, so after the operator closes the pane they cannot. That is the point of the change, and it is why the operator decides §6.

## 6. Decisions for the operator

1. **Should a message the operator typed be sent after they closed the pane?** The recommendation is yes: they typed it to be sent, and "Send now" or Cancel remain in the pane for as long as it is open. The alternative is to keep holding until the pane opens (today).
2. **A delay.** Send at once when the API is back, or wait (say 30 s) so a flapping connection does not send into a failing one. Recommended: the hold rule's own 20 s `unknown` wait, nothing more.
3. **Scope of the first version:** parked blocks only (recommended), or also blocks in tabs that no window has mounted, which needs windows to report what they have mounted.
