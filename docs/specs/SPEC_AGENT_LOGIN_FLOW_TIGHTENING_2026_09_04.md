# SPEC — Tighten the agent-pane login flow: auto-unblock on external bind, "Bind account" button

**Date:** 2026-09-04
**Status:** proposed — analysis complete, not yet implemented.
**Repo:** agentmuxai/agentmux
**Trigger:** Operator report, two related pieces of friction in the same flow:
1. Starting an agent shows the login-required prompt. The user switches to the
   Armory and binds an account for that provider (e.g. via the existing
   Bind-to-Agent context menu). The agent pane does **not** notice — it is
   still showing the same blocking prompt, and the user is stuck until they
   manually retry.
2. When no account is bound yet, the only recovery actions offered are a
   fresh login ("Log in"/"Login Again"), "Login via terminal", and a link to
   the Armory. If the user already has a **different, already-authenticated**
   account for the same provider (e.g. two Claude accounts, one already
   signed in), there is no one-click way to just use it — they have to leave
   the pane, go bind it from the Armory, and come back (and today, per issue
   #1, "come back" doesn't even self-resolve).

---

## 0. Read this first — a closely related bypass was just closed

`docs/analysis/ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md` (merged
2026-08-31, PR #2878) hardened a real invariant:

> **INV-PC (per-channel auth):** an agent running in channel *C* must launch
> only against a credential that was authenticated **in channel *C***.

As part of that work, **"Use existing login"** — a failure-row action that
copied the user's personal `~/.claude` credential into the agent's isolated
dir — was deleted outright (`failure-accessory.ts:125-132`, comment: *"copied
the user's personal `~/.claude` credential into this agent, which defeated
per-channel isolation"*).

**This spec's issue #2 is not that feature.** "Use existing login" read from
the ambient, un-scoped `~/.claude` file — a credential nobody chose and that
was never authenticated *for this channel*. This spec's "bind to existing
account" reads from **`db_accounts`**, the per-channel-scoped Armory account
list — a credential a human explicitly authenticated (in the Armory or in
another agent pane) *in this channel*, using the exact same link mechanism
(`LinkAgentIdentityCommand`) the Armory's own Bind-to-Agent menu already
uses. It is additive to INV-PC, not a regression of it — but anyone
implementing this must source the candidate list from the channel-safe path
(§3.3) or they will reopen a shape of the same bug.

## 1. What already exists (do not rebuild)

| Piece | Where | Notes |
|---|---|---|
| The one login CTA (post-consolidation) | `frontend/app/view/agent/failure/failure-accessory.ts` `failureToRow()`, `case "auth"` (~line 124) | Ships "Log in"/"Login Again" (primary, `on.loginAgain`), "🖥 Login via terminal" (`on.loginViaTerminal`), "Armory → Accounts" (`on.openArmory`). One surface since `PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02.md` (PR #2951) — see §2 below, this matters for where new actions go. |
| The state machine behind those buttons | `frontend/app/view/agent/hooks/useAgentControllerStatus.ts` | Owns `canRetry` (gates `useAgentCommands`'s unauthenticated-send fast-fail), `relogin()`, `loginViaTerminal()`, `existingAccountIdFor(providerId)` (this agent's *own* last-linked account — not a general provider search), `recordRecoveryIntent`/`beginRecoveryFlow`/`endRecoveryFlow` (concurrent-recovery-flow guards, several past P0/P1s live here — read the doc comments at lines ~277-370 before touching this file). |
| Channel-safe account listing | `agentmux-srv/src/server/agent_handlers/identity.rs:147-161` (`COMMAND_LIST_IDENTITY_ACCOUNTS` / `listidentityaccounts`) | Backed by `state.id_store.identity_list(provider)` — `id_store` is the **per-channel** store (`SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md`). **This is the channel-safe path.** Frontend cache: `frontend/app/view/identity/identity-model.ts` (`loadAccounts()`, `subscribeAccountChanges(fn)`), already consumed the same way by `AgentLaunchModal.tsx` and the Armory's `identity-accounts-tab.tsx`. `frontend/app/store/launch-flow-state/types.ts:184` (`accountsForProvider(state, providerId)`) is the existing "accounts for a provider" filter over that same cache. |
| The NOT-channel-safe listing (avoid for this feature) | `identity.self.accounts` (`app_api/mod.rs`) | Per `ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md` §6, this one still falls back to the global cross-channel mirror (`resolve_account`, not `resolve_account_for_spawn`) for continuity reasons unrelated to this feature. An account it lists can still fail to spawn with `MissingCredentials`. **Do not use this RPC to build the candidate list in §3.** |
| Bind action | `LinkAgentIdentityCommand` → `agent_handlers/identity.rs:590-611` | Upserts `db_agent_identity_links` (`ON CONFLICT(agent_id, provider) DO UPDATE`) — binding an agent that already has a link for this provider **replaces** it. Already used by the Armory's Bind-to-Agent context menu (`SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md`, PR #2485, implemented) and by the per-agent Identity tab. |
| Live-apply after a bind, for a *running* pane | `SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md` §2 | Two best-effort steps after the link upsert: (1) `SetMetaCommand` refreshing `cmd:env`'s provider config-dir env var to the newly-bound account's dir, (2) `ControllerResyncCommand{forcerestart:true}` — a persistent controller's already-running CLI process reads env only at spawn, so a forced (session-preserving, `--resume`) respawn is required or the new binding silently doesn't apply until a manual restart. |
| Bind-change event, scoped per agent | `format!("agentidentities:changed:{agent_id}")` broker event — emitted at `agent_handlers/identity.rs:601-607` (link), `:636-642` (unlink), `:556-562` (cascaded on account delete) | **Fires today on every bind, from every entry point** (Armory context menu, per-agent Identity tab, and this spec's new button). **Nothing in the agent pane currently listens to it.** This is the gap issue #1 closes. |
| Prior art for "bind to an existing account from a failure state" | `docs/specs/SPEC_ACCOUNT_ADOPTION_PIGGYBACK_LOGIN_2026_08_09.md` | Designed a near-identical failure-row "Use existing account" action for a *different* trigger (an unlinked legacy agent, not "no login yet"). **Status: superseded** — the chosen direction at the time was Armory-side-only (the Bind-to-Agent menu), on the reasoning that the failure row already routes users to the Armory. This spec revisits that call because live usage now shows the round-trip (leave pane → Armory → back) is real friction, and because issue #1's auto-unblock removes the main cost of offering it in both places (a stale prompt after binding elsewhere). Reuse this spec's candidate-set rules (§2.3) and adopt-action shape (§2) directly — they were correct and remain so; only the "where does it launch a picker from" chrome (§3.2) is new here. |

## 2. Issue #1 — auto-detect a bind and unblock the pane

### 2.1 What "stuck" means concretely

Two states currently never re-check anything after first render:

- **Pre-launch**: `runLaunchFlow` returns `auth_failed` → `canRetry(true)` →
  the synthetic `turnAttempted: false` auth failure row renders (per
  `PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02.md` Phase 1). `canRetry`
  also fast-fails any send attempt (`useAgentCommands`). Nothing clears
  `canRetry` except a click on "Log in" (`relogin()`).
- **Post-turn-failure**: a real 401/403 mid-turn → backend `FailureClass::Auth`
  → `state.failure` set (`code: "auth", turnAttempted: true`). Nothing clears
  it except a click on "Login Again", "Login via terminal", or Dismiss.

In both states, if the user goes and binds a usable account for this
provider **anywhere else in the app**, this pane has no way to find out.

### 2.2 Fix — subscribe to the per-agent bind event while blocked

In `useAgentControllerStatus.ts`:

1. While `canRetry()` is true **or** `failure()?.code === "auth"`, subscribe
   to `agentidentities:changed:<agentDefinitionId>` (same `agentDefinitionId`
   already resolved in `existingAccountIdFor`, via
   `getBlockMetaKeyAtom(opts.blockId, "agentId")`). Use the same
   `waveEventSubscribe`/broker-subscription primitive `identity-model.ts`
   already uses for `identityaccounts:changed` — this is a straight repeat of
   an existing pattern, not a new one.
2. On fire, re-run whatever auth pre-flight check the mount-time launch flow
   already performs (the same check that produced `auth_failed` in the first
   place — do not invent a second, possibly-divergent check; call the same
   function). Scope this re-check to the **current channel** (it must use the
   channel-safe path — see §1's `id_store` row — so a bind from a *different*
   channel's Armory instance, if that's even reachable, can't false-positive
   this).
3. If the re-check now succeeds: clear `canRetry`, clear/replace
   `state.failure` (the row disappears or updates — see §2.3), and let the
   sends fast-fail guard re-open (it already reads `canRetry`/`failure`, no
   change needed there).
4. If it still fails (e.g. the bind was for a different provider than this
   agent needs — links are keyed `(agent_id, provider)`, so this should be
   rare but is not impossible if a previous mismatched link existed): do
   nothing, leave the row as-is.

### 2.3 Explicit non-goal: do not auto-retry a turn

"Free the user to work" means **unblocking send**, not **auto-resubmitting
anything**. Do not call `relogin()`/`retryLastTurn` automatically from the
event handler in §2.2.

This is deliberate, not a missed opportunity, for the same reason
`retryAfterLogin: false` exists at all
(`PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02.md` §2, "A"): a stale
resend of an old message the user never asked to re-send is the exact bug
class this codebase has already paid down twice (the pre-launch bar's
original stale-resend bug, and `inFlightRetryAfterLogin`'s two P1s on PR
#2951). An external bind event, observed asynchronously, is a strictly
*less* trustworthy signal of "the user wants this turn re-sent right now"
than a direct button click — so it gets *less* license to act, not the same.
Concretely:

- Pre-launch case: clearing `canRetry` is enough — the user types and sends
  normally, no stale message exists to accidentally resend.
- Post-turn-failure case: clear the blocking aspect of the row (or downgrade
  it — e.g. keep a dismissible, non-blocking "Signed in — click Retry to
  resume" state reusing the existing retry button) but require the explicit
  click that already exists for actually re-running the failed turn.

### 2.4 Apply the standing design rules from the last time this file changed

`PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02.md` §"Standing design
property" recorded five rules earned from two P1s on the *previous* change to
this exact state machine. They apply verbatim to this new entry point:

1. Guard before you write (don't leave `inFlightRetryAfterLogin` or similar
   set by a call that no-ops).
2. Enumerate every flow that reaches the shared handler this fix touches —
   this is a **fourth** entry point into recovery state (`relogin`,
   `loginViaTerminal`, `/login`, and now "external bind detected") — update
   the enumeration comment at `recordRecoveryIntent` accordingly.
3. Capture what you need before any `await`/teardown.
4. Prefer carrying intent explicitly over re-deriving it after the fact.
5. Every entry point must derive shared intent (e.g. `turnAttempted`) the
   same way the others do.

## 3. Issue #2 — "Bind account" button, replacing "Armory → Accounts"

**Decided (2026-09-04, operator):** "Login via terminal" stays, unconditionally
— it is the only working path for providers whose in-app OAuth can't complete
in-app (`SPEC_HOST_CLI_LOGIN_CAPTURE_2026_06_20.md` §5.5 — Claude Code
v2.1.x's self-driving login TUI is the concrete example already in this
codebase), so making it conditional on candidate-account availability (this
spec's original draft) traded a rare "had to use a terminal" annoyance for an
occasional hard dead end. Instead, the new action takes the slot **"Armory →
Accounts" occupied** — but only when it has something to offer instead of that
link (see below); this resolves the open decision that was §3.6 in the prior
draft of this spec.

### 3.1 What changes in `failure-accessory.ts`

In the `case "auth":` arm (~line 124-150):

- **When ≥1 candidate account exists (§3.3):** replace "🗄 Armory → Accounts"
  with **"Bind account"** — named to match the vocabulary the Armory side
  already shipped ("Bind to Agent" context menu,
  `SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md`), so the same action
  reads as one concept from both directions instead of a new one. Dynamic
  label, mirroring the existing "Log in"/"Login Again" swap-by-state pattern:
  - Exactly one candidate → **"Bind: *\<account name>*"** (e.g. "Bind:
    work-claude") — binds directly on click, no picker.
  - More than one candidate → **"Bind account"** — opens the picker (§3.2).
  - Icon: reuse the `vault` FontAwesome icon `openArmory` used (`icon:
    "vault"`) — same visual language, since this button now owns the
    "go interact with your Armory accounts" affordance in this state.
- **When zero candidates exist: keep "🗄 Armory → Accounts" exactly as it is
  today.** There is nothing to one-click bind to yet — the user's actual next
  step is still "go create/authenticate an account," which is what that link
  is for. Dropping it here would remove the only path to the Armory from this
  row for the one case that most needs it, trading a real regression for
  visual tidiness. (`openArmory` is reused verbatim by other failure classes
  — `usage_limit`, `spawn_failure` — those are untouched either way.)
- **Keep "Login via terminal" exactly as it is today, always shown,
  regardless of candidate count.**

Resulting row for `code: "auth"`: `[Login Again / Log in]` (primary) ·
`[Login via terminal]` · `[Bind: <name> / Bind account]` **or**
`[Armory → Accounts]` (exactly one of these two, chosen by candidate count) ·
`[Details]` (when there's expandable content) · `[×]` dismiss. Net effect:
the row never grows past its current width — "Bind account" and "Armory →
Accounts" occupy the same slot, never both at once — it only gets more useful
in the case where a one-click fix is actually available.

### 3.2 Picker chrome for multiple candidates

`PaneRowAction` (`components/PaneRow.tsx`) is a flat button —
`onClick: () => void`, no submenu. For the single-candidate case this is
sufficient (label the button with the account name, bind directly on click).
For the multi-candidate case, do not extend `PaneRowAction` with new menu
plumbing — reuse the **same** `ContextMenuModel`/`ContextMenuItem`
(`submenu`/`checked`/`sublabel`) primitive the Armory's Bind-to-Agent context
menu already uses (`SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md`
§1), anchored at the button's position on click. This keeps "pick one of N
accounts to bind" as one component with one set of tests instead of two.

### 3.3 Candidate set — reuse the piggyback spec's rules verbatim

From `SPEC_ACCOUNT_ADOPTION_PIGGYBACK_LOGIN_2026_08_09.md` §2.3, unchanged:

- Source: the **channel-safe** cache (`identity-model.ts` / `loadAccounts()`
  / `accountsForProvider`, backed by `ListIdentityAccountsCommand` →
  `id_store.identity_list` — see §1's table; **not** `identity.self.accounts`).
- `resolve_provider_alias(account.provider) == resolve_provider_alias(agent.provider)`,
  canonicalized on **both** sides (the alias-mismatch class from Codex's P1
  on PR #2377 applies verbatim).
- OAuth-class accounts only (`secret_ref.backend == "oauth_config_dir"`).
  API-key accounts don't gate spawns the same way and are out of scope for
  this button (they're already handled by direct linking elsewhere).
- Sort `status == "valid"` first, then most-recently-updated; show the status
  dot (an expired account is still selectable — adopting then re-logging is
  still fewer steps than fresh OAuth — but visibly marked).
- Exclude the account already linked to *this* agent for this provider
  (nothing to adopt).

### 3.4 Adopt action

Identical to `SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md` §2 (this
is the same bind, from a different entry point — do not reimplement it):

1. `LinkAgentIdentityCommand { agent_id, account_id, provider }`.
2. Best-effort live-apply for a running pane: `cmd:env` config-dir refresh,
   then `ControllerResyncCommand{forcerestart:true}`.
3. Wrap in `beginRecoveryFlow()`/`endRecoveryFlow()` like every other
   recovery action in `useAgentControllerStatus.ts`, so the concurrent-flow
   guards (§2.4) apply uniformly.
4. **Do not auto-retry the failed turn from here either** — same reasoning as
   §2.3. The bind fires `agentidentities:changed:<agent_id>`, which issue
   #1's listener (§2.2) picks up and uses to clear the blocking state; the
   user (or the row's own now-relevant Retry button) drives anything further.
   This is the intended convergence: issue #2's button doesn't need its own
   unblock logic, because issue #1 already provides it for *any* bind source.

### 3.5 Non-goals (repeated from the piggyback spec, still correct)

- **No silent auto-adoption.** Even with exactly one valid candidate, require
  the click — an agent silently switching to an account the user never chose
  is the attributability failure `PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md`
  §7 exists to prevent.
- No cross-channel or cross-machine adoption (accounts are channel-local by
  design, per §1's `id_store` row).
- No API-key-class accounts through this button (out of scope, see §3.3).

### 3.6 Decided (was: open decision for the operator)

Resolved 2026-09-04 — see the callout at the top of §3. "Login via terminal"
is kept, always. "Armory → Accounts" is what makes room for "Bind account",
and only when there's a candidate to bind (§3.1) — not a conditional
narrowing of the terminal fallback.

## 4. Test plan

**Issue #1:**
- Hook test: while `canRetry()` is true, firing `agentidentities:changed:<agentId>`
  for this pane's own agent id and a now-passing pre-flight check clears
  `canRetry` and does **not** call `relogin`/retry.
- Hook test: same event for a **different** agent id is ignored.
- Hook test: while `state.failure.code === "auth"`, the same event downgrades/clears
  the blocking row without dispatching a retry.
- Regression: `useAgentCommands`'s existing `canRetry`-gating tests
  (`useAgentCommands.test.ts:186,203,326`) still pass unchanged — the fix only
  adds a new writer of `canRetry(false)`, it doesn't change what reads it.
- Live: reproduce the operator's exact report — start an agent, hit the login
  prompt, bind an account from the Armory's Bind-to-Agent menu in a different
  window, confirm the pane's prompt clears **without switching back to it**
  (subscription must fire while the pane is mounted but not necessarily
  focused).

**Issue #2:**
- Unit (candidate-set filter): mirrors
  `SPEC_ACCOUNT_ADOPTION_PIGGYBACK_LOGIN_2026_08_09.md` §5's plan verbatim —
  alias canonicalization both directions, oauth-class-only, exclusion of the
  already-linked id, status-then-recency sort.
- Unit (`failureToRow`): "Login via terminal" present in all three cases
  below (never conditional). Zero candidates → "Armory → Accounts" present,
  no bind action. One candidate → "Bind: \<name>" present, no "Armory →
  Accounts". 2+ candidates → "Bind account" (picker-opening) present, no
  "Armory → Accounts".
- Unit (adopt action): link RPC called with canonical provider; `cmd:env` +
  forced-resync fired only when the pane is currently open/running; recovery-
  flow counter begun/ended; **no** automatic retry dispatched (assert the
  retry/relogin mock is never called from this path — the regression this
  spec is most likely to accidentally reintroduce).
- Live: two Claude accounts in one channel, one already valid. New agent hits
  the login prompt, "Bind: *work-claude*" appears (replacing "Armory →
  Accounts") and is clickable in one step, next message runs without a fresh
  OAuth.
- Live: zero existing accounts for the provider — confirm "Login via
  terminal" **and** "Armory → Accounts" both still render and still work,
  unchanged.

## 5. Explicit non-goals for this spec

- Finishing the `disposable_test`/Store B per-account-scope split tracked in
  `ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md` §1 (out of scope,
  tracked separately).
- Building the `usable_in_channel` listing field
  (`ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md` §6) — not needed here
  because this spec deliberately sources candidates from the already-channel-
  safe `id_store` path (§1), not from `identity.self.accounts`.
- Any change to the inline transcript 401/403 CTA (surface C, per
  `PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02.md` §2.1) — it is a jump to
  the same row, not a fourth surface, and is unaffected by either fix here.
