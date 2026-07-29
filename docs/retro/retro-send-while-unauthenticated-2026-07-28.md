# Retro: sending a message while already known-unauthenticated triggers a doomed "Working…" round-trip instead of immediate feedback

**Date:** 2026-07-28
**Severity:** P1 — directly reproduces the user-facing symptom the whole `agenta/remove-auto-login-trigger` effort (PR #2318) was supposed to eliminate, in a code path that effort never touched.
**Status:** Root-caused and reproduced by direct code reading (not yet fixed — diagnostic retro only, per this repo's established retro convention).

---

## 1. What happened

A fresh portable build (0.54.6, post PR #2327/#2328) showed the mount-time "Log in"
button — the pane was already, correctly, in a known-unauthenticated state
(`status.canRetry()` true, per `launch-flow.ts`'s Phase 2 `auth_failed` bail). While
that button was still showing, the user typed a message ("u there?") and sent it.
The "Log in" button then disappeared, the pane entered a long "Working…" phase, and
only after a real wall-clock delay did the pane surface the user's message followed
by an in-conversation error: `Not logged in · Please run /login`.

The user's objection, verbatim: *"if I type 'u there' it already knows it is logged
out, that should be immediate feedback."* Correct — the pane had already proven
`canRetry() === true` before the message was ever typed. Nothing about sending a
message should have needed a slow round-trip to learn a fact the UI already knew.

## 2. Ground truth (verified independently, not just from the investigating
agent's report — every claim below was re-confirmed by direct `grep`/read
before writing this retro)

- **The composer has zero auth gating.** `grep -n "canRetry\|authStatus\|agentReady\|CheckCliAuth" frontend/app/view/agent/hooks/useAgentCommands.ts frontend/app/view/agent/components/AgentFooter.tsx` returns **no matches at all**. Neither the send button, the Enter-key handler, nor `deliverToBackend` (`useAgentCommands.ts:397-485`) references any of the three signals `useAgentControllerStatus.ts` already exposes for exactly this purpose.
- **The "Log in" bar is a sibling, not a gate.** `agent-view.tsx:1535` (`<Show when={status.canRetry()}>`, the retry bar) and `agent-view.tsx:1746` (`<AgentFooter .../>`) are independent elements in the same render tree — the footer is never wrapped in, or disabled by, the retry-bar's condition. The textarea has no `disabled` binding at all.
- **A real controller already exists by the time the pane's own auth check would matter.** `agent-model.ts:270-275` (and the parallel path at `:585-590`) calls `RpcApi.ControllerResyncCommand(TabRpcClient, { ..., forcerestart: true })` **unconditionally**, at agent-launch time, with no auth check anywhere nearby in that function. This runs *before* the pane's own `launch-flow.ts` Phase 2 auth check ever executes for that mount. So even though Phase 2 correctly bails with `"auth_failed"` and (per its own doc comment, `launch-flow.ts:34-38`) deliberately skips Phase 3's controller registration, a controller was already registered earlier by the unrelated launch path. `agentmux-srv/src/server/agent_handlers/input.rs`'s `get_controller(&cmd.blockid)` therefore **finds** a real controller and proceeds — the "no controller registered" fast-fail this session's own `TurnStartFailed` comment explicitly names as a case it handles never fires here, because there IS a controller.
- **The identity-injection gate only blocks structural failures, not "needs reauth."** `agentmux-srv/src/identity/resolver/inject.rs:449` injects the OAuth config-dir env var **unconditionally** once a binding resolves to an `OAuthConfigDir` secret ref. The subsequent on-disk expiry probe (`inject.rs:460-472`) only *updates the account's status field* to `needs_reauth` for UI display (publishing `identityaccounts:changed` so the Armory tab refreshes) — it never blocks the injection or the resulting spawn. The gate's own test, `inject_oauth_class_probes_and_flips_status_to_needs_reauth` (`inject.rs:1432+`), documents this explicitly: "env injection still happened... the CLI launches with the dir env var set and will trigger OAuth itself when it sees no tokens."
- **The backend never re-runs the same check the frontend already ran.** `agentmux-srv/src/server/agent_handlers/input.rs`'s `COMMAND_AGENT_INPUT` handler never calls `CheckCliAuthCommand` — the exact RPC `launch-flow.ts`'s Phase 2 already called to *produce* the `canRetry() === true` state in the first place. The backend spawns the CLI on a credential it could have known was stale, using a check that already exists elsewhere in the same codebase.
- **Consequence:** `AgentInputCommand` (`useAgentCommands.ts:433-437`) **succeeds** — it does not throw. `deliverToBackend`'s `catch` (`useAgentCommands.ts:440-471`), which dispatches the `TurnStartFailed` action added earlier in this same session (PR #2318), never runs, because there was no synchronous RPC failure to catch. The pane sits in the optimistic `TurnStart` state (dispatched by `handleSendMessage` before the RPC ever ran) until the real, successfully-spawned CLI process makes its own network call, fails auth on the provider's side, and streams the error back through the normal conversation/output pipeline — which is the actual wall-clock delay the user experienced as "Working…".

## 3. Root cause, in one sentence

Two independent gaps compound: **nothing on the frontend checks already-known auth
state before allowing a send**, and **nothing on the backend re-verifies auth before
spawning a real CLI process on a credential it already has reason to distrust** —
so a message typed into a pane that is visibly showing "Log in" travels all the way
down to a real subprocess spawn before anything says no.

## 4. How this was missed — the part the user specifically asked for

This session did substantial, careful work on the auth state machine: removing the
silent auto-login trigger, making the mount-time flow notify-and-wait instead of
retry-forever, adding `retryAfterLogin` gating to `relogin()`'s success paths, and —
directly relevant here — adding the `TurnStartFailed` reducer action
(`frontend/app/store/agent-pane-state/{types.ts,reducer.ts}`) specifically so a send
that never actually started a turn wouldn't leave the pane stuck in "Working…"
forever.

That last piece is exactly why this gap is easy to miss: **it looks solved.**
`TurnStartFailed`'s own doc comment (`types.ts`, and mirrored in
`useAgentCommands.ts:454-460`) explicitly lists *"the identity spawn gate blocking
on a bad credential"* as one of the synchronous-failure cases it was built to
handle. I wrote that comment. I believed the spawn gate would catch a bad
credential — including an expired/`needs_reauth` one — because "spawn gate blocks
bad credentials" was true in the general case I'd tested (a **missing** account
row, or a structurally wrong `SecretRef` shape — both of which genuinely do throw
synchronously and are correctly caught by `TurnStartFailed`). I never checked
whether an account that **exists but has a stale token** — precisely the case a
`canRetry()`-true pane is already sitting on — hits the same gate the same way. It
doesn't: `inject.rs`'s gate only validates *shape* (does a binding exist, is the
`SecretRef` the right variant), not *validity* (is the token inside still good).
The on-disk expiry probe that DOES check validity runs one line later, and its only
effect is a status-field update for the UI — never a block.

In other words: the fix I shipped this session correctly handles "the turn never
started because the RPC rejected it," and I reasoned by analogy that "a bad
credential" was a subset of that case without re-deriving it from the actual gate
code. It wasn't. A **known-expired** credential and a **missing** credential hit
completely different code paths in `inject.rs`, and only one of them fails fast.
The state-reducer work this session was scoped to *login flows* (mount-time launch,
explicit relogin) — it never considered *"the pane already knows it's logged out;
what happens if the user ignores that and sends anyway"* as a distinct entry point
needing its own guard, because from inside the login-flow code, that entry point is
invisible — it bypasses the login flow entirely and goes straight through the
ordinary send path.

## 5. Recommendations (not implemented — diagnostic retro only)

1. **Client-side pre-send guard.** `handleSendMessage`/`AgentFooter`'s send path
   should check `status.canRetry()` (and/or `status.authStatus() === "unauthenticated"`)
   before ever dispatching the optimistic `TurnStart` or calling `deliverToBackend`.
   On a known-bad auth state, either block the send with an inline "please log in
   first" affordance, or — if sends should still be queued for after login — make
   that an explicit, visible decision rather than the current silent fall-through.
2. **Backend fast-fail using the check that already exists.** `input.rs`'s
   `COMMAND_AGENT_INPUT` handler should consult the same signal `launch-flow.ts`'s
   Phase 2 already uses (a fresh `CheckCliAuthCommand`, or at minimum the account's
   cached `needs_reauth` status from the last probe) before spawning, and return a
   fast, structured error — not a successful `Ok(None)` that lets a doomed subprocess
   spawn anyway. This closes the gap for every caller of `AgentInputCommand`, not
   just this one composer path.
3. **Regression test for the actual gap.** Neither of the two fixes above currently
   has a test scenario that: (a) puts a pane in `canRetry() === true`, (b) sends a
   message, (c) asserts either the send is blocked client-side or the backend
   returns a fast, typed error — no CLI spawn, no "Working…" window. This exact
   scenario should have a name and a test before it's considered fixed, not just a
   patch.
4. **Re-audit `TurnStartFailed`'s own doc comment.** Its list of "synchronous
   failure" cases (`useAgentCommands.ts:454-460`) should be corrected to note that
   a stale/`needs_reauth` credential is currently NOT one of them — either fix the
   gate so it becomes true, or fix the comment so the next person doesn't make the
   same reasoning-by-analogy mistake this retro documents.

## 6. Related docs

- `docs/specs/REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md` — the
  original investigation this session's `retryAfterLogin`/`TurnStartFailed` work
  came out of; describes a different "stuck Working…" mechanism (optimistic
  `TurnStart` never reverted) that this retro's bug is a sibling of, not a repeat.
- `docs/retro/retro-my-agents-fresh-channel-regression-2026-07-27.md` — the
  immediately preceding retro this session, which found a structurally similar
  pattern (a signal that already exists — the registry's validation state there,
  the CLI auth-check here — not being consulted at the point where it would have
  prevented the user-visible symptom).
- PR #2318 (`fix(auth): remove mount-time auto-login trigger, fix login
  persistence and turn-state races`) — introduced `TurnStartFailed`; this retro
  documents the boundary of what that fix actually covers.
- `agentmux-srv/src/identity/resolver/inject.rs`'s own test module — already
  documents the "env injection still happens, needs_reauth is display-only"
  behavior this retro treats as the backend half of the root cause; it was written
  as an accurate description of *current* behavior, not flagged there as a gap.
