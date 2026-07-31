# SPEC — Agent-pane mount-time auth check validates the wrong directory

**Date:** 2026-07-31
**Type:** Bug-fix spec
**Scope:** `frontend/app/view/agent/hooks/useAgentControllerStatus.ts`,
`frontend/app/view/agent/flows/launch-flow.ts`
**Status:** Diagnosed, fix scoped — needs a go-ahead before implementation
**Trigger:** User report, live-testing a `task dev` build: opening agent "Parko"
showed a "Log in" prompt even though the agent's actual bound account was
already authenticated — clicking "Log in" produced no visible re-auth (no
browser, no OAuth prompt) and the agent worked immediately after. User noted
this is roughly the 4th time this exact symptom shape has come up.
**Related:** `docs/specs/REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md`
(names 3 separate credential systems + 5 login code paths; this bug lives
inside system 1, "provider-CLI identity"), `docs/reports/REPORT_AGENT_AUTH_DIVERGENCE_2026_06_20.md`
(a different, previously-fixed divergence in the same family — two panes on
the same identity resolving `CLAUDE_CONFIG_DIR` independently),
`docs/retro/retro-agent-auth-relogin-noop-2026-07-01.md` ("Login Again" as a
silent no-op — same *symptom*, different root cause than this one),
`docs/retro/retro-login-three-code-paths-2026-07-20.md` /
`docs/specs/PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md` (partial
consolidation via PR #2255, doesn't touch this specific check).

---

## 1. Root cause — two independently-resolved directories that structurally disagree

Two things happen on every agent-pane mount, computed by two unconnected code
paths:

**A. The auth CHECK (frontend-driven, runs at mount):**
1. `useAgentControllerStatus.ts:214-226` (`buildAuthEnv`) calls
   `getApi().ensureAuthDir(prov.id)` — keyed **only by provider id** (e.g.
   `"claude"`), nothing agent- or account-specific.
2. That resolves, host-side, to `agentmux-cef/src/commands/platform.rs:74-79`'s
   `ensure_auth_dir`: the shared, account-wide default,
   `~/.agentmux/shared/providers/<provider>/` — the function's own doc comment
   already says *"the per-identity bundle override (identity_handlers) still
   wins for explicit multi-account"*, i.e. this code already knows it's not
   the final word, but nothing downstream corrects for that at this call site.
3. `launch-flow.ts:222-226` calls `CheckCliAuthCommand` against that generic
   dir. If it looks unauthenticated there, `needsLogin = true` →
   `launch-flow.ts:277-285` sets phase `"auth-expired"` or `"first-login"` and
   surfaces the "Log in" button.

**B. The real SPAWN (backend-driven, runs when the CLI actually launches):**
1. `agentmux-srv/src/identity/resolver/inject.rs:430-449`
   (`inject_identity_env`'s OAuth-class branch) reads the dir straight from
   the agent's **actual bound account row**:
   `SecretRef::OAuthConfigDir { dir }`, persisted per-`account_id` at the time
   that account was created.

**These two dirs are only the same by coincidence.** Any agent with a real,
explicit account binding (the normal case for a returning user) has its own
per-account dir from (B) — but (A) never looks it up, so it always checks the
generic shared default from (B)'s perspective, an increasingly-stale or
never-populated location once per-account isolation is in play. The mount
check fails, the pane shows "Log in," and clicking it invokes `relogin()`
(`useAgentControllerStatus.ts:474`) → for Claude specifically,
`headlessLoginUrlUnsupported: true` (`providers/catalog.ts:57`) skips straight
to `seedGlobalLogin` (`flows/run-provider-login.ts:309-339`) — a sub-second
file copy of the *already-valid* global `~/.claude` credential into the
checked (generic) dir, then a controller restart. No real authentication
happens; the click just patches the generic dir so the *next* check passes,
which is why the agent works immediately after with no visible login step.

## 2. Why this isn't the fix for the fix — and doesn't need one

The obvious question: is there already a per-account-aware read path this
should call instead? Yes, verified directly (not inferred):

- `identity.ensureaccountdir` (`agentmux-srv/src/server/identity_handlers.rs:55,152-169`)
  is an **existing, already-wired RPC** taking `{ provider_id, existing_account_id }`
  and returning `{ account_id, dir }`.
- Its handler calls `compute_and_ensure_account_dir`
  (`agentmux-srv/src/server/identity_auth_dirs.rs:159-`), which resolves/creates
  **the exact same per-account dir** `inject_identity_env` reads at spawn time
  (both key off the same `account_id` → `SecretRef::OAuthConfigDir { dir }`
  row).
- The frontend **already fetches this agent's linked account id** for this
  provider today — `launch-flow.ts:254-263`
  (`RpcApi.ListAgentIdentitiesCommand` → `links.find(l => l.provider === provider.id)?.account_id`)
  — but only *after* `needsLogin` is already true, purely to pick the
  "expired" vs. "first-login" wording. It's the right lookup, just called too
  late to matter for the check itself.

So this is **not** a "build a new resolution path" problem, and it does
**not** need the not-yet-built CLI-provider slice of the Credential Broker
(§3 below) — every piece needed already exists, just in the wrong order.

## 3. Does this need the Credential Broker / reducer-consolidation work?

Asked directly this session: this bug is the *same class* of problem this
session's other tracked work is about (Process Broker, `TurnPhase` reducer
sprawl) — multiple independent mechanisms answering "is this thing valid"
that can silently disagree — but it is a **different, sibling domain** (auth,
not process/turn liveness), already independently diagnosed in
`REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md`, and it is
**not blocked on, and does not need to wait for**, that report's target
"one Credential Broker" architecture (§6.1). Checked directly: the actual
`CredentialBroker` code that exists today (`agentmux-srv/src/broker/scheduler.rs`,
`RefreshScheduler`) is explicitly scoped to MuxBus only — its own module doc
(`agentmux-srv/src/broker/mod.rs:12-27`) says so — provider-CLI identity is
still an unbuilt future phase there. Unlike Process Broker (which has a
shipped Phase A to extend in Phase B), there is no equivalent shipped slice
for CLI-provider identity to route this fix through yet. Building that slice
just to fix this one check would be a large, unrelated lift; the fix in §4
below is fully self-contained and doesn't preclude that broader work
happening later — if anything, it's a smaller-scoped instance of exactly the
consolidation §6.1 argues for in general (one resolution path instead of two).

**No existing tracking issue/discussion covers this specific gap** — checked;
nothing analogous to Discussion #2375 (process/turn-liveness) exists for auth
architecture. Closest are issue #678 (general identity system) and PR #2255
(open, unrelated to this specific divergence). Worth a decision on whether to
open one now (see §6).

## 4. Proposed fix — resolve the linked account before checking, not after

Move the existing `ListAgentIdentitiesCommand` lookup (`launch-flow.ts:254-263`)
earlier, and use its result to resolve the real per-account dir *before*
building `authEnv`, instead of only using it for post-hoc wording:

1. In `useAgentControllerStatus.ts` (or wherever the launch flow first has
   `agentDefinitionId` + `provider.id` in scope), call
   `RpcApi.ListAgentIdentitiesCommand` to get this agent's linked
   `account_id` for this provider, if any.
2. If a linked account exists, call `identity.ensureaccountdir` with
   `{ provider_id, existing_account_id: linkedAccountId }` and use **its**
   returned `dir` for `authEnv` (in place of `ensureAuthDir`'s generic
   default).
3. If no linked account exists (genuine first-time), fall back to today's
   `ensureAuthDir` behavior unchanged — there's nothing to override yet, and
   this is the one case where the generic default is actually correct.
4. `launch-flow.ts:254-263`'s existing lookup becomes redundant with step 1's
   result — thread it through instead of calling
   `ListAgentIdentitiesCommand` twice per mount.

This changes zero backend code — both RPCs it uses already exist and are
already exercised elsewhere. The change is entirely in how the frontend
sequences and reuses calls it already makes.

## 5. Test plan

1. Unit-test-level: a hook/flow test asserting that when
   `ListAgentIdentitiesCommand` returns a linked account for the provider,
   `identity.ensureaccountdir` is called with that account id and its
   returned dir is what ends up in `authEnv` / `CheckCliAuthCommand`'s
   `auth_env` — not `ensureAuthDir`'s.
2. Unit-test-level: when no linked account exists, confirm the fallback to
   `ensureAuthDir` is unchanged (no regression for genuine first-login).
3. Live `task dev`: open an agent with an already-authenticated, explicitly
   bound account (the exact repro condition) — confirm the pane goes
   straight to ready, no "Log in" prompt, no notification.
4. Live `task dev`: a genuinely first-time/unauthenticated agent still shows
   "Log in" as before (regression check on the fallback path).

## 6. Open question for go-ahead

Two independent decisions, not coupled to each other:

1. **Implement §4's fix now?** Small, self-contained, no backend changes, no
   architecture dependency — ready to build immediately if you want to
   proceed the same way as the Process Broker Phase B work.
2. **Open a tracking discussion for the broader auth-architecture
   consolidation** (analogous to #2375), so the next time a bug in this
   family surfaces it's tracked in one place instead of re-diagnosed? This
   fix doesn't require that discussion to exist first — it's a separate,
   longer-horizon question about whether to formally track §6.1's "one
   Credential Broker" work the same way Process Broker is tracked.

---

*Diagnosis and fix scoping only. No files changed except this spec.*
