# Agent Pane Mount/Auth Notifications & Launch-Auth Reducer

**Date:** 2026-07-26
**Author:** AgentA
**Status:** implemented — PR #2304 (LaunchAuthState never-silent mount notifications); verified in code 2026-08-10.
**Depends on / relates to:**
- [`SPEC_LAUNCH_AUTH_STATE_MACHINE_2026_05_14.md`](SPEC_LAUNCH_AUTH_STATE_MACHINE_2026_05_14.md) — the **pre-launch modal's** `AuthState` reducer (bundle/identity selection before a pane exists). Sibling machine, not the same one — see §7.
- [`PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md`](PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md) — the three-tier `runProviderLogin` fallback this spec builds notifications on top of.
- [`REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md`](REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md) — describes `agentmux-srv/src/broker/state.rs`'s `CredentialState` scheduler, a **different, currently-unrelated** system (MuxBus cloud credentials only today; CLI-provider auth explicitly deferred to that plan's "Phase C+"). This spec must not collide with that future convergence — see §7.
- PR #2300 (`agenta/deterministic-login-ux`) — shipped the `LaunchPhase` footer-label system and the `onLoginSuccess` visible-confirmation mechanism this spec extends. Everything in that PR stays; this spec adds to it, not replaces it.

---

## 1. Problem

Manually testing PR #2300 surfaced two related gaps, both traced to the same root cause: **the agent-pane mount flow (`agent-view.tsx`'s `onMount` → `useAgentControllerStatus.startLaunchFlow` → `launch-flow.ts`'s `runLaunchFlow`) treats a brand-new agent and a resumed agent identically, and most of what it does is invisible to the user.**

Specifically, as of PR #2300:

1. **A login can open with zero warning.** `agent-view.tsx`'s `onMount` calls `status.startLaunchFlow()` unconditionally — there is no `isContinue`/resume distinction anywhere in that file. (This doc intentionally cites no line numbers for code on PR #2300's branch: that branch is still under active review and every fix shifts them — the landmark text below is the stable reference, not a coordinate.) If a previously-working agent's cached credential has gone stale, Phase 2's `CheckCliAuthCommand` reports `authenticated: false` and the flow goes **straight into `runProviderLogin`** — a browser tab or terminal window can appear with no prior message in the pane's conversation. The only textual trace is `log("auth", "not authenticated — starting login flow...")` inside `launch-flow.ts`'s `if (needsLogin) {...}` block, which routes to the activity-log/shell-terminal channel per `AgentDocumentView.tsx`'s own header comment — **not** the visible conversation the user is looking at. This reads as "the app randomly popped open a login."

2. **Nothing confirms what happened on an ordinary resume.** The common case — reopening an agent that's already authenticated — produces exactly one line, `log("auth", "authenticated as X (method)")`, into that same hidden channel. `onLoginSuccess` (the callback that posts a permanent, visible "✓ Logged in as X" node into the conversation, `agent-view.tsx`'s `onLoginSuccess` handler passed to `useAgentControllerStatus`) is scoped inside `launch-flow.ts`'s `if (needsLogin) {...}` block and structurally **cannot** fire on this path — confirmed by reading, not inferred. Phase 3's resume-vs-fresh distinction (`if (status === "init") {...} else if (status === "done" || status === "running") {...}`) is likewise only ever `log("agent", ...)`'d into the hidden channel — a user reopening a long-running agent gets no on-screen confirmation that they're looking at a resumed conversation versus a fresh one.

3. **No single source of truth for "what phase is this pane in."** Today's signals are scattered: `LaunchPhase` (PR #2300, footer label only), `needsLogin`/`loginWaiting` booleans, the `onLoginSuccess` callback, and Phase 3's inline `if/else`. This is exactly why the `onLoginSuccess`-not-wired-to-`relogin()`/`loginViaTerminal()`/`useGlobalLogin()` gap (fixed as a same-day follow-up to #2300) was easy to miss in the first place — there was no one place to check "does every terminal state actually notify the user."

## 2. Goals

- **G1 — No silent login.** A login attempt (opening a URL, a terminal, or seeding from a global credential) must never be the *first* thing the user sees. A visible message announcing *why* a login is about to happen must land in the pane conversation before any such action starts.
- **G2 — Every mount narrates itself.** Every agent-pane mount posts a minimal, permanent record into the conversation: what kind of mount this was (fresh vs. resumed), whether auth needed anything, and the resulting ready state — not just transient spinner text that vanishes once loading finishes.
- **G3 — One reducer, not scattered booleans.** Consolidate `LaunchPhase` + `needsLogin`/`loginWaiting` + `onLoginSuccess` + Phase 3's init/done branch into a single, explicit state machine with a transition table, so "does every terminal state notify the user" is answerable by reading one table, not five files.
- **G4 — Keep PR #2300.** `skipTier1` determinism, the existing `LaunchPhase` footer labels, and the cancel-login button all stay as-is; this spec's reducer supersedes `LaunchPhase` as the source of truth but preserves its exact label text and timing.

## 3. Non-goals

- **Not** touching the pre-launch modal's `AuthState` reducer (`SPEC_LAUNCH_AUTH_STATE_MACHINE`) — that machine governs bundle/identity selection *before* a pane exists. This spec's reducer governs *after* a pane exists, mount → ready. They stay siblings.
- **Not** wiring against `agentmux-srv/src/broker/state.rs`'s `CredentialState` scheduler. Per `REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md`, CLI-provider credentials are explicitly out of scope for that system until its "Phase C+" — this spec's reducer is frontend-only and provider-agnostic on purpose, so it doesn't need to be unwound when that phase eventually lands.
- **Not** addressing the unrelated ~9-10 minute dev-instance crash (tracked in `docs/retro/retro-agentmux-srv-9min-crash-2026-07-26.md`, PR #2301).
- **Not** blocking the expired-token relogin on user confirmation — decided in §8 Q2 to notify-then-proceed, matching the rest of this work's "automatic but never silent" approach rather than adding a confirmation gate.

## 4. Current flow (baseline)

```
mount
  │
  ▼
startLaunchFlow() ── unconditional, no fresh/resume distinction ──────────┐
  │                                                                       │
  ▼                                                                      (nothing
Phase 0: container check (silent unless container agent)                  visible
  │                                                                        posted to
  ▼                                                                        the pane
Phase 1: resolve/install CLI (silent unless install needed)               conversation
  │                                                                        anywhere
  ▼                                                                        in this
Phase 2: CheckCliAuth ──────────────┬── authenticated:true ──► Phase 3    whole
  │                                  │   (log() only, hidden)              column)
  │ authenticated:false              │
  ▼                                  │
runProviderLogin() ──────────────────┘
  │  tier1/2/3 — a URL, terminal, or seed can fire HERE, first thing the user sees
  ▼
recheck → onLoginSuccess (conversation node) ── ONLY reachable from this branch
  │
  ▼
Phase 3: ControllerResync + GetControllerStatus
  │
  ├─ status "init"  → log("agent", "ready...")           (hidden)
  └─ status "done"  → log("agent", "previous turn...")    (hidden)
```

## 5. Proposed flow

```
mount
  │
  ▼
Phase 0/1 (unchanged, silent unless install needed — Q3)
  │
  ▼
Phase 2: CheckCliAuth
  │
  ├─ authenticated:true ──► no notification here — folds into Phase 3's line (Q1)
  │                          ──► Phase 3
  │
  └─ authenticated:false ──► has this agent run before? (`blockData?.meta?.["cmd"]`
      │                       already set — cheap, synchronous, no new RPC — see §8 Q6)
      │
      ├─ never run before ──► POST (neutral, not alarming — this is completely
      │                        expected for a brand-new agent, nothing "expired"):
      │                        "Signing in to Claude…"
      │
      └─ ran before ────────► POST WARNING (this is the actually-surprising case):
                               "⚠ Your Claude login has expired — signing back in…"
                              (either branch: + Cancel, reusing PR #2300's cancel-login button)
                             │
                             ▼
                          runProviderLogin() ── now the FIRST visible action
                          was the warning above, not the popup itself
                             │
                             ▼
                          onLoginSuccess → "✓ Logged in as X" (existing, unchanged)
  │
  ▼
Phase 3 → the ONE end-of-flow line, sourced directly from GetControllerStatus's
  existing shellprocstatus — this IS the fresh/resume signal, no separate
  up-front classification step needed. THREE values are possible
  (STATUS_INIT/STATUS_RUNNING/STATUS_DONE in agentmux-srv's persistent
  controller — reagent P1 on PR #2303 caught that "running" was missing
  from the original draft, which meant a persistent controller resumed
  while still alive/mid-turn stayed completely silent, not just unstyled):
  "Ready — type a message to start"          (status "init", fresh)
  "Resumed — continuing where you left off"  (status "done" OR "running" — both
                                               mean this agent has a turn on
                                               record, "running" if anything a
                                               STRONGER resume signal than "done")
```

Two design decisions baked into this diagram (both resolved in §8, not left open):
- The `authenticated:false` warning is posted **before** `runProviderLogin()` is called, not after — satisfying G1 by ordering, and it **proceeds automatically** after posting (Q2) rather than blocking on a confirmation click; the existing cancel button is the escape hatch.
- The smooth path gets exactly **one** conversation line, at the end, not one at start and one at end — the footer's spinner labels already cover "something is happening" in real time (Q1). There's no separate early "Starting…"/"Resuming…" state either: fresh-vs-resume isn't actually knowable until Phase 3's `GetControllerStatus` call returns, so there was never a real moment to post it any earlier than the existing end-of-flow line.

## 6. Proposed reducer: `LaunchAuthState`

Mirrors the existing `AuthState` pattern (`frontend/app/view/agent/auth/auth-state.ts`) — a pure `update(state, event) → {state, notification?}` function, with `notification` describing what (if anything) to post into the conversation. Lives in the frontend (`frontend/app/view/agent/flows/launch-auth-reducer.ts`, new), consistent with `AuthState` being frontend-only. Side effects (the actual RPC calls, opening terminals) stay in `launch-flow.ts`/`useAgentControllerStatus.ts`, exactly as `auth-flow-controller.ts` is separate from `auth-state.ts` today.

`LaunchPhase` (PR #2300) is **subsumed** by this reducer's `kind` field — every existing `LaunchPhase` variant becomes a `LaunchAuthState.kind` value with the same footer-label text; nothing about the footer changes for a user.

| `kind` | Meaning | Conversation notification | Footer label (unchanged from PR #2300 where applicable) |
|---|---|---|---|
| `resolving-cli` | Resolving/installing the CLI (existing) | none unless install needed (Q3) | resolving-cli |
| `checking-auth` | Running `CheckCliAuth` | none (instant, <1s typical) | checking-auth |
| `auth-ok-quiet` | Authenticated, no login needed | none — folds into the Phase-3 line below (Q1) | (clears immediately) |
| `first-login` | **New.** Never run before (`meta.cmd` unset — Q6), needs its first login | "Signing in to Claude…" (neutral) + Cancel | checking-auth → opening-login-terminal |
| `auth-expired` | **New.** Ran before (`meta.cmd` set), but the token is now stale | "⚠ Your login has expired — signing back in…" + Cancel | checking-auth → opening-login-terminal |
| `waiting-for-login-link` | tier 1 URL-capture (existing) | existing `AuthUrlBox` | unchanged |
| `opening-login-terminal` | tier 2/3 (existing) | "Opening a terminal for login…" | unchanged |
| `waiting-for-login-completion` | (existing) | (existing) | unchanged |
| `verifying` | Post-seed/terminal-success one-shot recheck (existing) | none (transient, <10s) | verifying |
| `login-success` | (existing `onLoginSuccess`) | "✓ Logged in as X" (existing, unchanged) | ready |
| `resumed-ready` | **New.** Phase 3 `status === "done"` **or `"running"`** (a persistent controller resumed while still alive/mid-turn is at least as much a resume as "done" — reagent P1 on PR #2303, "running" was missing from the original draft) | "Resumed — continuing where you left off" | ready |
| `fresh-ready` | **New.** Phase 3 `status === "init"` (the only value meaning "never run") | "Ready — type a message to start" | ready |
| `failed` | (existing) | existing failure banner + Retry | failed |

No separate `classifying`/`starting`/`resuming` states — see §5's note on why fresh-vs-resume is only knowable at Phase 3, not before.

Every row has a notification column filled in and a stated reason when it's intentionally quiet — that's the concrete answer to G3: a reviewer can scan this one table and confirm no terminal state was *accidentally* left silent, instead of re-deriving it from five files the way this gap was found.

## 7. Relationship to other state machines (avoiding collisions)

- **`AuthState` (pre-launch modal):** governs bundle/identity selection before a pane exists at all. `LaunchAuthState` never runs concurrently with it — by the time `LaunchAuthState`'s first state (`resolving-cli`) starts, `AuthState` has already reached its terminal `ready` and the pane has been created. No shared state, no need to unify them into one machine — the naming ("Launch*" vs "Auth*") should stay distinct enough not to imply they're the same reducer.
- **`CredentialState` (MuxBus broker, `broker/state.rs`):** unrelated today. **Naming decision (§8 Q4):** the new reducer is named `LaunchAuthState`, not "auth broker" — even though that's the term used when this spec was first requested — specifically to avoid future confusion with the *actual* `CredentialState` broker once its Phase C+ work lands and potentially does start covering CLI-provider credentials.

## 8. Decisions (resolved 2026-07-26)

| # | Question | Decision | Why |
|---|---|---|---|
| Q1 | Should the ordinary "already authenticated" resume post its own visible text? | **No.** It folds into the single Phase-3 line (`resumed-ready`/`fresh-ready`). | Avoids double-texting the highest-frequency case (open a working agent, see one line, not two back-to-back). The footer's existing `LaunchPhase` labels already give real-time feedback while Phase 2 runs. |
| Q2 | When a token has expired on resume: block on confirmation, or notify-then-proceed? | **Notify-then-proceed**, automatically, with the existing cancel button as the escape hatch. | Matches this whole effort's direction (deterministic + automatic + visible, not gated behind clicks). The original complaint was about a login opening *unannounced* — ordering (warn, then act) resolves that without adding a new confirmation step nothing else in this flow has. |
| Q3 | Should Phase 0/1 (container check, CLI install) get their own notification lines? | **No** — silent unless something goes wrong, matching today. | Rarely the interesting part of a resume; PR #2300 already covers CLI-install progress via a separate mechanism. |
| Q4 | What should the reducer be called? | **`LaunchAuthState`** / `launch-auth-reducer.ts`. | Avoids colliding with the name of the actual `CredentialState` broker (§7) — that name is reserved for the system this spec explicitly stays out of. |
| Q5 | Does "resumed" need a stronger signal than a text line (composer placeholder, a divider)? | **Out of scope for v1** — text line only. | Revisit once the text-only version is in use and it's clear whether it reads clearly enough on its own. |
| Q6 | The `auth-expired` wording ("Your login has expired") is wrong for a brand-new agent's very first login — nothing expired, it never had one. How to tell the two apart? | Check `blockData?.meta?.["cmd"]` (already read synchronously at the top of `runLaunchFlow`, already the established signal elsewhere in this file for "has this agent's CLI ever been resolved before" — e.g. `resolveCliForRecovery`'s doc comment). Unset → `first-login` ("Signing in to Claude…"); set → `auth-expired` ("⚠ …has expired…"). | `meta.cmd` is written once, right after Phase 1's first successful CLI resolve, and persists on the block across remounts — it's a free, accurate, zero-new-RPC proxy for "has this agent completed at least one launch before," which is exactly what determines whether "expired" is true or misleading. |

## 9. Acceptance criteria (for whenever this moves past draft)

- [ ] No login-related window (URL open, terminal spawn) can appear without a preceding conversation-visible message in the same pane, on every one of the three auto-login call sites (mount, relogin, `/login`) and the two explicit-action paths (`loginViaTerminal`, `useGlobalLogin`).
- [ ] Every `LaunchAuthState.kind` in §6's table has an explicit, tested notification behavior — including the states that are decided to be intentionally quiet (§8 Q1/Q3) — verified by a test that asserts the table itself, not just spot-checked flows.
- [ ] Resuming an agent with prior history visibly says so; starting a fresh one visibly says so; the two are distinguishable from the conversation alone.
- [ ] A brand-new agent's first-ever login never says "expired"; an agent that has run before and lost its credential does. Both still notify before the login attempt (Q6).
- [ ] `LaunchPhase`'s existing footer-label timings (from PR #2300) are unchanged — this is additive, not a rework of the timing work already shipped.

## 10. References

- PR #2300 — deterministic login UX, `LaunchPhase`, `onLoginSuccess`. This doc intentionally cites no line numbers for code on that PR's branch (still under active review, so any number would go stale the next commit) — the quoted condition/function text is the stable reference instead.
- `frontend/app/view/agent/agent-view.tsx` — `onMount`'s `status.startLaunchFlow()` call (mount site) and the `onLoginSuccess` handler passed into `useAgentControllerStatus`.
- `frontend/app/view/agent/flows/launch-flow.ts` — Phase 2's `if (needsLogin) {...}` block and Phase 3's `status === "init"`/`"done"`/`"running"` branch (see §5's update — `"running"` was missing from the original draft, reagent P1 on PR #2303).
- `frontend/app/view/agent/components/AgentDocumentView.tsx` — header comment documenting the log()-to-hidden-channel routing this spec works around.
- `frontend/app/view/agent/auth/auth-state.ts` — the sibling reducer pattern this spec's `LaunchAuthState` mirrors.
- `docs/retro/retro-agentmux-srv-9min-crash-2026-07-26.md` — lands via PR #2301 (sibling PR, not yet merged to `main` as of this spec's own review — it exists on that PR's branch).
