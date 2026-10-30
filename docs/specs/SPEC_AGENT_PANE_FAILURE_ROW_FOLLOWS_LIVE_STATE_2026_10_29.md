# Spec: the agent pane's failure row follows the agent's live state

**Status:** active — approved by the operator (relayed by AgentX, 2026-10-29). Kanban #10592 spec / #10593 build (narko); the build is in PR #5874.
**Date:** 2026-10-29
**Author:** Manoz@Area54
**Verified against:** `agentmux` `main` @ `58d7cc7b2` (2026-10-29), by reading code. Nothing here is built, and the operator's exact 429 case is **not reproduced** (§2.3).
**Builds on:** `SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md`, `SPEC_AGENT_AVAILABILITY_ONE_STATE_2026_10_26.md` (#4565), `SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md`.
**Touches the same files as:** Loap-3's rename of "Bind account" to "Use account…" (#10566): `failure-accessory.ts`, `bind-account-candidates.ts`. Build after that merges; this spec changes no label.

## 1. Ask

The operator: after an error, the pane's failure row should follow the agent's live state. Today a 429 shows the failure row with Bind/Connectors buttons; the service heals; the row never changes, although typing works. They asked for "new framework thinking level": a model, not another patch.

## 2. What happens today

### 2.1 Where the row comes from, and what clears it

The row is the pane reducer's `state.failure`. It is set by three things and cleared by three:

| Sets it | Note |
|---|---|
| the `agentfailure` event (`useAgentFailure.ts`) | srv's classified failure of a non-zero exit (`agents/failure.rs`) |
| **mount**: `agent:last_failure` in the block's meta, read once | no age check, no look at the agent's state; srv keeps that meta for the usage-limit and sign-in classes because it routes work by it |
| the synthetic "Not signed in" row (`synthetic-row.ts`) | only for an agent that never ran |

| Clears it | Note |
|---|---|
| the next `TurnStart` | so typing works and the row goes: the only heal today |
| `FailureCleared`: Dismiss, a login, `useAuthHealth` (auth only) | the auth class has a health check; no other class has any |
| (nothing else) | no time, no srv state, no successful request by anything but this pane's own next turn |

For a transient class (`rate_limited`, `overloaded`, `network`) the row arms an auto-retry ladder (5 rungs, about 3.9 minutes) and then waits for a click. If the service heals during or after the ladder, nothing tells the row.

### 2.2 Which classes offer account actions

`failure-accessory.ts`: `auth` offers Login Again, Login via terminal and Bind/Connectors; `usage_limit` offers "Accounts (switch / upgrade)" (Connectors); every other class offers Retry (or nothing). srv classifies a message with "rate_limit", "rate limited" or HTTP 429 as `rate_limited` *before* the usage-limit and sign-in checks (`failure.rs`), so a fresh 429 gets Retry only.

### 2.3 How a 429 ends up with account buttons (not reproduced)

I could not find a path where a *new* 429 gets Bind/Connectors. The candidates, to be settled by reading one block's `agent:last_failure` when it recurs:

1. **A stale row.** The row on screen is an older `auth` or `usage_limit` failure re-seeded from meta at mount (§2.1); a 429 happened since but did not replace it (a failure that arrives while a different one shows replaces it, so this needs the 429 to be a retry inside a turn, not a classified exit).
2. **A different class.** The CLI reports a provider quota as a 429 whose text lacks the rate-limit keywords and names the plan limit; it classifies as `usage_limit`, whose row has the Connectors button, and "usage limit" is what the operator then calls the 429.
3. **The spawn gate.** The agent's account was marked limited or signed out by an earlier episode; the gate's refusal text classifies as `auth`.

The model below makes all three harmless, whichever it was.

## 3. Proposal: a failure has a live verdict

The record (`AgentFailure`, with the time it happened) stays the **cause**. What the row shows is the cause **judged against what is true now**, in one pure function the row, the Swarm chip and the Kanban already share the idea of (`unavailableOfMeta`, `agent-unavailable.ts`, which mirrors srv's `unavailable_of_block`).

### 3.1 Inputs (all already exist)

- the failure: `code`, `at`, `resetsAtMs`;
- the agent's live state from srv (`agentruntime` feed, `AgentRuntime.state`, from `availability`): `limited`, `signed_out`, or can work (`idle`, `working`, `waiting_on_user`);
- the pane's own last completed turn (`turn-ended`, outcome `completed`) and its time;
- for `network`: the connectivity reachability of the pane's API (already used by `reconnect`).

### 3.2 The verdict

| Verdict | When | The row |
|---|---|---|
| **Active** | the cause still holds: `auth` and srv says `signed_out`; `usage_limit` and srv says `limited` (reset not passed); a transient class with no completed turn since `at` and the ladder not yet ended | as today, with the class's own actions |
| **Healed** | a turn completed after `at`, or srv's state moved to can-work for an `auth`/`usage_limit` cause, or the API answers again for `network` | cleared at once (`FailureCleared`), no click, nothing left but the transcript |
| **Lapsed** | a transient class whose ladder ended with no heal | a muted one-line note ("Rate-limited 12 min ago") with Retry and Dismiss, no error colour, no account action; it goes with the next completed turn or a Dismiss |

### 3.3 Which classes may offer account actions

Only a cause that is about the **account**, while srv still says so:

| Class | Account actions | When |
|---|---|---|
| `auth` | Login, Use account…, Connectors | srv says `signed_out`, or there is no account bound |
| `usage_limit` | Use account… (switch), Connectors | srv says `limited` |
| `rate_limited`, `overloaded`, `network`, `killed`, `no_output`, `unknown_non_zero`, `max_turns`, `context_exceeded`, `spawn_failure` | **never** | |

A rate limit that is really the plan's quota arrives as `usage_limit` (§2.3 case 2) and is then judged by srv's `limited`, with its reset. A row whose verdict is Healed or Lapsed offers none.

### 3.4 Mount

A persisted failure is **judged before it is shown**: re-seeding from meta shows only an Active one. A pane that was closed while the cause held, and opened after it healed, shows nothing; one opened while it holds shows the row, with no flash of a row that then clears.

### 3.5 The pane that was open before the heal

The row is a derivation, so it subscribes to the live state, not to a poll: when srv's state for the agent leaves `limited`/`signed_out`, or `turn-ended` completes, the verdict flips and the row clears while the pane is open. This is the case the operator hit.

## 4. Where it goes

- A pure `failureVerdict(failure, live, now)` next to `failure-accessory.ts`, with the table of §3.2–3.3 as its test; `failureToRow` gets the verdict and builds account actions only from `Active` account classes. The class → action map stays as it is, so Loap-3's rename merges without a conflict of meaning.
- `useAgentFailure.ts`: the seed at mount and the live subscription call the verdict; the `useAuthHealth` check stays as the source of "signed out is over" for `auth` when srv is not yet reporting.
- srv: nothing new. The `agentruntime` feed already carries the state; the one fact to confirm in the build is that it is published to a pane's window for a parked or foreground agent alike.

## 5. Tests

- A table test of `failureVerdict` over class × live state × completed-turn-after × ladder-ended.
- A 429 never yields an account action in any state; a `usage_limit` yields one only while srv says `limited`.
- A persisted `auth` failure for an agent srv reports as running shows nothing at mount.
- A pane open on a `limited` agent clears its row when the state moves to idle, with no click.
- The existing `failure-accessory` tests pass unchanged.

## 6. Decisions for the operator

1. **Which signal drives the row.** Recommended: srv's live state for account causes, "a turn completed since" for transient ones, and the failure record only as the cause. The alternative is to keep clearing on the next turn only (today).
2. **How a heal clears.** Recommended: at once and silently. The alternative is a short "Recovered" note first.
3. **What a transient row becomes once its retries are spent.** Recommended: the muted Lapsed note of §3.2. The alternative is to leave it as the error row until a turn completes (today) or to drop it with the ladder.
4. **Account actions.** Recommended: exactly §3.3; in particular a rate limit never offers Bind/Connectors or "Use account…".
5. **A persisted failure at mount.** Recommended: judged first (§3.4).
