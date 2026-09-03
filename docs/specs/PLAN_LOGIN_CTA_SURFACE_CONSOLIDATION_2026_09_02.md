# Plan — consolidate the agent pane's two (really three) separate login CTAs

**Date:** 2026-09-02
**Status:** implemented — Phases 1-3 shipped.
**Repo:** agentmuxai/agentmux
**Trigger:** User report — the agent pane shows *two separate login buttons*: a
blue "Log in" button, and a row of buttons that includes "Login Again". They
should be one thing.

**Scope note — this is about the CTA *surfaces*, not the login *implementations*.**
`docs/specs/PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md` covers the
orthogonal, still-open work of collapsing the four *login flow implementations*
(`runProviderLogin`, `useGlobalLogin`, `loginViaTerminal`, `AuthFlowController`)
onto one core. That plan is about what happens *after* a click. This plan is
about how many different things the user can click. The two can ship
independently; neither blocks the other.

---

## 1. What the user is seeing

Three distinct login call-to-action surfaces exist in the agent pane today, all
three of which ultimately call the **same** function — `status.relogin()`
(`frontend/app/view/agent/hooks/useAgentControllerStatus.ts:499`). They differ
only in chrome, placement, trigger condition, and one boolean argument.

| # | Surface | Rendered at | Looks like | Gate |
|---|---|---|---|---|
| **A** | The blue "Log in" bar | `agent-view.tsx:2176-2196` | Full-width bar, solid `var(--accent-color)` background, white text (`.agent-retry-btn`, `styles/_retry-empty.scss:17-31`) | `status.canRetry()` |
| **B** | The failure-recovery row | `agent-view.tsx:2253-2270` via `failure-accessory.ts:113-119` | `<PaneRow accent="error">` — a row of labelled buttons: 🔑 **Login Again** (primary) · 🖥 Login via terminal · 🗄 Armory → Accounts · Details · × | `failureAtom()` is set with `code === "auth"` |
| **C** | The inline transcript CTA | `virtualization/DocumentRow.tsx:307-317` | Small red outlined button, "Login Again →" (`.agent-error-login-btn`, `styles/_document-nodes.scss:1808+`) | An `agent_error` document node with `code ∈ {401, 403}` |

The user's "blue login button" is **A**; the "row of buttons" is **B**. **C** is
a third instance of the same intent that the report didn't mention (it only
appears inline next to a 401/403 transcript row, so it's easy to miss) but which
any consolidation must account for or it becomes the *next* duplicate.

Not in scope, because it isn't a CTA: `AgentAuthPanel` (`agent-view.tsx:2173`,
bottom-docked `InAppLoginPanel`) is the *in-progress* login session UI — the URL
box, paste-code field, Cancel / "Use terminal instead". It renders **after** a
login has started, and is the correct shared destination for all three CTAs. It
should stay exactly as it is.

## 2. Why they're separate — the actual history, not a guess

They are not redundant by accident. Each was added at a different time, for a
different trigger, by a different spec, and nobody ever unified the *display*
layer because each one's own gating condition was individually correct:

- **A (`canRetry`)** answers *"the mount-time launch flow bailed before the agent
  ever started."* It is set in exactly two places
  (`useAgentControllerStatus.ts:340` on `runLaunchFlow` returning `auth_failed`,
  and `:884` restoring itself after an unsuccessful `relogin`). At this point
  **no turn has ever been attempted** — which is precisely why its click handler
  passes `{ retryAfterLogin: false }` (`agent-view.tsx:2192`). There is no failed
  turn to re-run, and a comment at that call site records the bug that taught
  them: without the flag, a successful login on an agent with prior history
  silently re-sent its last *old* message as a new turn.
- **B (failure row)** answers *"a turn ran and the provider rejected our
  credentials."* It is driven by the backend's failure classifier
  (`agentmux-srv/src/agents/failure.rs`, `FailureClass::Auth`) arriving as an
  `agentfailure` event → `FailureObserved` → `state.failure`. Because a turn
  *did* fail here, its `relogin()` correctly takes the default
  `retryAfterLogin: true` and re-runs it. It also carries genuinely
  auth-specific extras A doesn't have: the terminal fallback, the Armory link,
  the expandable stderr/detail body, dismiss, and the auto-retry budget in
  `useAgentFailure.ts`. Spec: `SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md`.
- **C (inline node)** answers *"this specific message in the transcript is a
  401."* Added by `SPEC_REAUTH_FROM_AUTH_ERROR` §7 so the fix is reachable from
  where the error is *read*, without scrolling to a banner.

So the split is historical accretion around three genuinely different *trigger
conditions* — but the **action is identical** (`relogin()`), and the user
correctly perceives them as the same button drawn three ways.

### 2.1 They can be on screen simultaneously

This is the part that makes it a real defect rather than a style inconsistency.
`canRetry` and `failure` are independent signals with no mutual exclusion:

- `useAgentFailure` seeds its row on mount from the persisted
  `agent:last_failure` block meta (`useAgentFailure.test.ts:124` pins this), so a
  pane reopened after an auth failure shows **B** immediately.
- The same mount runs the launch flow, which hits the same bad credential,
  returns `auth_failed`, and sets `canRetry(true)` — showing **A**.

Result: a pane that failed auth, was closed, and reopened shows a blue "Log in"
bar *and* a red failure row with "Login Again", stacked, both wired to the same
function. Nothing in the code prevents this; no test asserts against it.

An earlier draft of this section claimed the two were mutually exclusive
(citing `useAgentControllerStatus.ts:219-220`, "mid-turn auth failures don't set
`canRetry`"). That citation is accurate but covers only the **live** path; it
does not constrain the **mount** path above. The two surfaces are separate
sibling `<Show>`s (`agent-view.tsx:2176` and `:2253`) with no gating between
them.

#### Reproduced live (manoz, 2026-09-02) — observed, not inferred

Run against a dev instance on main **without** this change, on an agent whose
Claude login was genuinely expired (so the mount-time pre-flight check really
failed and really set `canRetry`). The other half was staged by writing
`agent:last_failure` directly to that block's meta via `object.UpdateObjectMeta`
(`code: "auth"`, `retryable: true`) — the exact state
`core::persist_last_failure` leaves behind after a real mid-turn 401 — then
forcing a remount. Sampled 3× at 6s intervals, identical every time:

```
bars: 1  barText: ["Log in"]
rows: 1  rowText: ["🔐 Authentication failed / auth · retryable /
                   🔑 Login Again / 🖥 Login via terminal /
                   Armory → Accounts / ▸ Details / ×"]
```

Both surfaces on screen simultaneously and **stable** — not a transient flash.

Two things that repro establishes, beyond "it happens":

1. **The seed is mount-only, and that is load-bearing.** Staging the meta
   against an *already-mounted* pane changed nothing; the stacking required a
   close-and-reopen. That is precisely why this survived so long: it is not
   visible while you sit in the pane watching it, only on reopen — which is
   also when a user is least able to report it precisely.
2. **In the stacked state the ROW is the richer surface and the BAR is the
   redundant one** (the row carries terminal fallback, Armory, Details, dismiss;
   the bar carries one button). So collapsing onto the row is not merely
   deduplication — it strictly preserves the better of the two. This is the
   answer to "why keep the row rather than the bar".

## 3. Goals

1. **One login CTA visible at a time**, in one visual language.
2. **Preserve the `retryAfterLogin` distinction.** This is the one piece of real
   behavioural difference and it must survive consolidation — a
   never-started-agent must not re-send an old message.
3. Preserve the auth-specific secondary actions (terminal fallback, Armory,
   details, dismiss) that only B has today.
4. No regression to the in-progress login UI (`AgentAuthPanel`) or to the
   auto-retry budget in `useAgentFailure`.

## 4. Proposed approach — make B the single surface, delete A, keep C as a jump

**Adopt the failure row (`PaneRow`) as the one login CTA.** It is already the
richest surface, already the shared accessory primitive used by the session
digest / ActivityDock / fork bar, and already carries the secondary actions. A
is a bespoke one-off bar with its own SCSS that exists only because it predates
the failure row.

### Phase 1 — give the pre-launch case a failure row instead of its own bar

The blocker to simply deleting A is that `canRetry`'s trigger (`runLaunchFlow`
→ `auth_failed`) does not currently produce a `state.failure`, so deleting the
bar would leave that case with **no** CTA at all.

1. On the `auth_failed` branch (`useAgentControllerStatus.ts:339-342`), dispatch
   a `FailureObserved` carrying a synthetic `AgentFailure` with
   `code: "auth"` — mirroring the shape the backend classifier already emits for
   the mid-turn case (`failure.rs:206`/`:276`), with wording specific to
   "never started" (e.g. title "Not signed in", detail explaining the agent
   hasn't launched yet).
2. Thread the `retryAfterLogin` distinction through the failure itself rather
   than through the button. Add an optional field to the pane's failure state
   (e.g. `turnAttempted: boolean`, default `true` for backend-classified
   failures, `false` for this synthetic pre-launch one). `agent-view.tsx`'s
   `onLoginAgain` then reads it:
   `void status.relogin({ retryAfterLogin: failure.turnAttempted })`.
   This is the load-bearing step — it is what lets one button serve both cases.
3. `failureToRow`'s `"auth"` arm gets a label conditional on the same field:
   "Log in" when `!turnAttempted`, "Login Again" when `turnAttempted`. (The
   secondary actions — terminal, Armory — are correct for both cases as-is.)

### Phase 2 — delete surface A

4. Remove the `<Show when={status.canRetry()}>` block (`agent-view.tsx:2176-2196`)
   and the `.agent-retry-bar` / `.agent-retry-btn` rules
   (`styles/_retry-empty.scss:7-31`).
5. **Keep the `canRetry` signal itself.** It is not only a display gate — it is
   read by `useAgentCommands` to fast-fail sends while unauthenticated
   (`useAgentCommands.test.ts:186`, `:203`, `:326` all pin this behaviour) and by
   `/login` (`commands/global/login.ts:67-71`). Only its *rendering* goes away.
   This is the highest-risk misstep available in this plan: deleting the signal
   along with the button would silently re-enable sending messages into an
   unauthenticated agent.
6. Audit the `:884` "restore the mount-time Log in button" branch — its comment
   and its `!retryAfterLogin` scoping both describe a button that will no longer
   exist. The signal still needs restoring (for #5's gating), but the comment
   must be rewritten or it will mislead the next reader.

### Phase 3 — make C a jump, not a duplicate

7. Leave the inline 401/403 CTA in place (it has real value at the point of
   reading), but change it from a second entry point into a **scroll-to** for the
   failure row — or, if the row is guaranteed visible whenever C is, remove C's
   button entirely and let the row be the only actionable thing. Decide this from
   a live repro: if the two are always co-visible, drop C's button; if C can
   appear without a row (e.g. a 401 node from a *prior* session in scrollback
   with no live failure), keep it and have it call the same handler.

## 5. Alternative considered and rejected

**Keep both, add mutual exclusion** (`<Show when={status.canRetry() && !failureUI.row()}>`).
One line, zero risk, fixes the stacking. Rejected as the *end state* — it leaves
two visual languages for one action and leaves the third surface untouched, so
the next person to touch auth UI still finds three places to change. It is,
however, a reasonable **stopgap to land first** if the double-button is
user-visible today: it is independently correct and does not conflict with
Phases 1-3.

## 6. Testing

- Reducer/hook: a synthetic pre-launch `FailureObserved` produces a row whose
  action label is "Log in" and whose `relogin` call passes
  `retryAfterLogin: false`; a backend auth failure produces "Login Again" with
  `retryAfterLogin: true`. This is the invariant the whole plan rests on.
- Regression: `useAgentCommands`'s existing `canRetry`-gating tests must still
  pass unchanged after Phase 2 (proves the signal survived the button's deletion).
- A pane-level test that the auth CTA appears **exactly once** in the two
  scenarios that today produce two: (a) mount after a persisted auth failure,
  (b) mid-turn 401 on an agent that also has `canRetry` set.
- Manual: reopen a pane whose agent previously failed auth and confirm one CTA,
  not a blue bar stacked above a red row.

## 7. Decisions taken during implementation

The two open questions were resolved as follows. Both are cheap to revisit —
each is one expression.

1. **Placement: the failure row's existing position, not the old bar's.**
   Surface A sat directly above the composer; the row sits in the pane's
   accessory stack. Keeping one component in one place is the whole point of the
   consolidation, and splitting the same row across two positions by case would
   have reintroduced a variant of the problem. If the never-launched case turns
   out to need more prominence, move the row — don't re-fork it.
2. **Accent: `"active"`, not `"error"`, for the never-signed-in case.** An
   agent that has simply never been signed in hasn't failed at anything, so red
   overstated it. `PaneRow`'s existing `"active"` accent is `var(--accent-color)`
   — literally the same blue the deleted bar used — so the pre-launch case keeps
   its original colour language while adopting the shared component.
   `FailureRow.accent` widened from `"error"` to `"error" | "active"` for this;
   every non-auth failure and every turn-attempted auth failure is unchanged.

### Standing design property: auth CTAs coexist, so shared state must be scoped

Recorded as a property of the design rather than a note about one bug, because
the next person adding an entry point will not read the PR thread.

Keeping surface C means **two auth CTAs can be live at once, by design** — a
persistent `agent_error` document node and a transient failure row, reachable
independently. That is correct and worth keeping (see below), but it has a
standing consequence: **any mutable state shared behind those entry points can
be written by one while another is in flight.**

Two P1s on this PR came from exactly that, and both were reached through
surfaces or flows their author had not enumerated:

- `inFlightRetryAfterLogin` was written *before* the in-flight guard, so a
  no-op'd call from one CTA corrupted the intent of a live flow started from
  another. Fixed by guarding first — both writers are now structurally
  identical.
- The same flag was assumed to be readable only by the flow that wrote it. It
  is not: `/login` opens the same `AuthUrlBox`, and therefore the same "Use
  terminal instead" handler, without ever writing it. Fixed by scoping the
  value to one flow (cleared in `endRecoveryFlow`, `false` at rest).

The rules that fall out, for anyone adding a fourth entry point or a new
recovery flow:

1. **Guard before you write.** A call that no-ops must not leave state behind.
2. **Enumerate the flows that reach a shared handler, not just its callers.**
   `useTerminalInstead` is reached by `relogin`, `loginViaTerminal` *and*
   `/login`; only the first two write the state it reads.
3. **Capture before teardown.** If a handler waits for a flow to end, read what
   it needs *before* the wait — ending the flow is what clears it.
4. **Prefer carrying intent explicitly over re-deriving it.** Every attempt on
   this PR to infer intent after the fact raced something that had already
   changed underneath it.
5. **Every entry point must derive the intent the same way.** `/login` reads the
   pending failure's `turnAttempted` exactly as the row's buttons do. When it
   did not, the same pending failure retried or didn't depending purely on
   which control the user happened to use — and that divergence is invisible
   from either call site alone.

### A test-process rule this PR earned

**Re-run mutation checks on EXISTING tests when you change shared-state
semantics, not only on new ones.**

Five results on this PR looked like evidence and were not. Four had familiar
shapes — an input path that never reached the app, a test asserting nothing, a
mutation that silently didn't apply, a proposed assertion that was
unsatisfiable. The fifth was new and is the reason for this rule: three tests
were written correctly, genuinely failed under mutation at the time, and then
**stopped measuring anything** when a later commit changed the state they
depended on. They kept passing while asserting nothing.

Mutation discipline alone does not catch that, because the check passed
honestly when it was run. The two defences that would have: shaping a test like
the real lifecycle rather than a convenient sequential approximation (the tests
in question awaited a flow to completion, where the real user clicks mid-flight),
and re-mutating existing tests after changing shared state.

### Phase 3 outcome — surface C was KEPT, deliberately

The plan left this to a live repro. Resolved from the code instead, which
answers it definitively: `agent_error` nodes are produced only by the live
stream parser (`stream-parser.ts:713`) and are **persistent document nodes**,
whereas the failure row is transient pane state cleared by the next `TurnStart`.
A 401 stays in the transcript long after its row is gone, so C is reachable
without a row and cannot be replaced by a scroll-to. It is not a redundant CTA;
it is the same action at a second, genuinely-different point in time. It always
means a turn *did* run, so its existing `relogin()` default
(`retryAfterLogin: true`) was already correct and is unchanged. The reasoning is
recorded at the call site so it isn't "consolidated" away by mistake later.

## 8. What actually shipped

- `PaneFailure.turnAttempted` (`agent-pane-state/types.ts`) + the matching
  optional field on the `FailureObserved` command, defaulted to `true` in the
  reducer so every backend-classified failure is unchanged.
- `agent-view.tsx`: a `createEffect` that raises a synthetic
  `turnAttempted: false` auth failure while `status.canRetry()` is set, and
  retracts *only that* synthetic row when it clears. Guarded so a real
  backend failure (richer: stderr tail, auto-retry budget) always wins.
- `failure-accessory.ts`: the auth arm's label ("Log in" / "Login Again") and
  the row accent both key off `turnAttempted`.
- `useAgentFailure`: `onLoginAgain` now takes `turnAttempted` and forwards it,
  so the label and `relogin`'s `retryAfterLogin` are sourced from one value and
  cannot disagree.
- Deleted: the `<Show when={status.canRetry()}>` bar and its
  `.agent-retry-bar` / `.agent-retry-btn` SCSS. **`canRetry` itself survives** —
  it still gates `useAgentCommands`'s unauthenticated-send fast-fail and is read
  by `/login`; the stale comment at its restore site was rewritten to say so.
- 13 new tests (6 accessory, 3 reducer, 4 hook). The hook tests were
  mutation-checked: hardcoding `onLoginAgain(true)` fails 2 of them.

### One deliberate behaviour change worth knowing about

The old blue bar could **not** be dismissed; the shared row can. The synthetic
pre-launch row is raised on `canRetry` *transitions* only (the effect reads the
current failure untracked), because tracking the failure as well would make
Dismiss re-raise the row instantly — an undismissable row. So dismissing the
"Not signed in" row leaves the pane with no auth CTA until `canRetry` next
transitions, which a *failed* login attempt does (it restores `canRetry`,
re-raising the row).

This is safer than it first looks, and the reason is load-bearing rather than
incidental (found by **manoz** reviewing this change). Attempting to send while
dismissed is not only blocked-and-logged: `useAgentCommands`' guard gates
`setAuthNotice` on `!authFailureToPreserve && !liveAuthFailure`, and for the
synthetic pre-flight failure both are false once the row is dismissed — so this
is precisely the case that notice fires for. The user gets visible feedback, and
`/login` still works. Dismiss is a real choice, not a dead composer, and it is
strictly more control than the old bar (which could not be dismissed at all).

Three alternatives to the canRetry-transitions-only design were considered and
rejected (manoz, independently, on review):

- **Track `failureAtom` as well** — reintroduces the undismissable row.
- **Re-raise on `TurnStart`** — fires too late; the send is already blocked by
  then.
- **Re-raise on composer focus** — surprising, and focus is not an auth event.

Neither of us found a fourth that keeps dismiss working *and* re-raises on a
better signal, so the shipped combination (canRetry transition + authNotice on a
blocked send) stands.

Because that behaviour depends on one condition staying false in exactly this
case, it is pinned by a test — `useAgentCommands.test.ts`, *"still surfaces a
visible authNotice after the pre-flight auth row is dismissed"* — so a later
simplification of that condition can't silently turn dismiss into a dead end.
Note the deliberate asymmetry with the mid-turn path (doc comment at
`useAgentCommands.ts:218-224`), where the guard re-dispatches the failure and
the row *does* reappear; the pre-flight row intentionally does not.

**A copy bug fell out of checking this.** That same notice used to read *"Not
logged in — click 'Log in' below to continue."* It fires only when
`state.failure` is null — i.e. exactly when **no** row is rendered. That was
correct while the standalone blue bar was always up in this state; with the bar
deleted it pointed at a button that no longer exists. Changed to name the
recovery that always works: *"Not logged in — run /login to sign in, then send
again."*
