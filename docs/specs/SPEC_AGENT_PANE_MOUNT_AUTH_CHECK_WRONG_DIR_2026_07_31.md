# SPEC — Agent-pane mount-time auth check validates the wrong directory

**Date:** 2026-07-31
**Type:** Bug-fix spec
**Scope:** `frontend/app/view/agent/hooks/useAgentControllerStatus.ts`,
`frontend/app/view/agent/flows/launch-flow.ts`
**Status:** Implemented (PR #2377, 5 fixup commits after codex review — see §4's
correction note) — awaiting a live `task dev` check before merge.
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
should call instead? Partially — verified directly, but the specific RPC
named below turned out to be the wrong one (see the correction, and §4's
matching correction note — codex caught the same stale claim twice, once
here and once in §4, since this section was never updated after §4 was):

- `identity.ensureaccountdir` (`agentmux-srv/src/server/identity_handlers.rs:55,152-169`)
  is an **existing, already-wired RPC** taking `{ provider_id, existing_account_id }`
  and returning `{ account_id, dir }`.
- ~~Its handler calls `compute_and_ensure_account_dir`
  (`agentmux-srv/src/server/identity_auth_dirs.rs:159-`), which resolves/creates
  **the exact same per-account dir** `inject_identity_env` reads at spawn time
  (both key off the same `account_id` → `SecretRef::OAuthConfigDir { dir }`
  row).~~ **Corrected:** `compute_and_ensure_account_dir` does **not** read
  the account row — it deterministically reconstructs
  `<identities>/<account_id>/<provider>/` from the account id alone and
  creates it if missing. For an account whose real credential lives at a
  non-canonical stored path, this can silently diverge from what
  `inject_identity_env` actually reads (`secret_ref.dir`). See §4's full
  correction note — the implemented fix uses `GetIdentityAccountCommand`
  instead, a pure read of the stored `secret_ref`.
- The frontend **already fetches this agent's linked account id** for this
  provider today — `launch-flow.ts:254-263`
  (`RpcApi.ListAgentIdentitiesCommand` → `links.find(l => l.provider === provider.id)?.account_id`)
  — but only *after* `needsLogin` is already true, purely to pick the
  "expired" vs. "first-login" wording. It's the right lookup, just called too
  late to matter for the check itself.

So this is **not** a "build a new resolution path" problem, and it does
**not** need the not-yet-built CLI-provider slice of the Credential Broker
(§3 below) — every piece needed already exists (once pointed at the right
RPC), just in the wrong order.

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

## 4. Implemented fix (updated 2026-08-01 — corrected after codex review)

**Status: implemented, PR #2377.** The original plan below called for
`identity.ensureaccountdir`/`ensureAccountDir()` to resolve the linked
account's dir. That RPC turned out to be the wrong one to reuse here — codex
caught it during review (see the correction note at the end of this section)
— so what actually shipped reads the account's own stored directory instead
of reconstructing one:

1. `launch-flow.ts` resolves this agent's linked `account_id` for the
   provider up front (moved earlier from where it used to run only after
   `needsLogin` was already true, purely for wording) — via
   `ListAgentIdentitiesCommand`, canonicalizing both sides of the comparison
   through `provider-id-aliases.ts`'s `canonicalProviderId` (a link row
   persisted under a legacy alias like `claude-code` must still match).
   When a migrated agent has BOTH a canonical and an alias row for the same
   provider, `lastLinkedAccountId` picks the *last* canonical-equivalent
   match — mirroring `inject_identity_env`'s own `HashMap::insert`
   last-write-wins precedence, not the first.
2. If a linked account exists, call `RpcApi.GetIdentityAccountCommand({id})`
   and read `secret_ref.dir` directly when `secret_ref.backend ===
   "oauth_config_dir"` — using it in place of `authEnv`'s generic dir for
   both `SetMetaCommand`'s `cmd:env` and `CheckCliAuthCommand`'s `auth_env`.
3. If no linked account exists (genuine first-time) or the lookup/read
   fails, fall back to today's `ensureAuthDir`-based `authEnv` unchanged.
4. The pre-existing `existingAccountIdFor` helper in
   `useAgentControllerStatus.ts` (feeding `relogin()`/`loginViaTerminal()`/
   `useGlobalLogin()`) had the identical strict-comparison bug — codex
   caught this as a second call site during review. `lastLinkedAccountId`
   was extracted into the shared `provider-id-aliases.ts` module so both
   call sites resolve "the account this agent uses for this provider"
   identically.
5. Codex's review also surfaced two backend gaps this frontend fix exposed
   (not introduced by it, but newly user-visible once the frontend started
   recognizing alias-bound links as valid): `inject_identity_env`'s
   def-provider gate compared raw (possibly aliased) provider strings
   against the canonical definition provider, so an alias-only-bound agent's
   already-successfully-injected credential was misclassified as "no account
   bound at all" and the spawn was blocked anyway; and the OAuth expiry
   probe (`probe_oauth_status`) only recognizes canonical provider strings,
   so an alias-bound account's status was silently never refreshed. Both
   fixed in `agentmux-srv/src/identity/resolver/inject.rs` alongside the
   frontend change, canonicalizing via the existing `resolve_provider_alias`.

**Correction (2026-08-01):** the original version of this section (below,
struck through) proposed reusing `identity.ensureaccountdir` /
`ensureAccountDir()` — the same RPC the seed-from-global recovery path uses.
Codex caught that this RPC's underlying `compute_and_ensure_account_dir`
**does not read the account row at all** — it deterministically reconstructs
`<identities>/<account_id>/<provider>/` from the account id and creates it if
missing. For an account whose real credential lives at a non-canonical
stored path (e.g. carried forward from a bundle-era migration), that
reconstructed path can silently diverge from what `inject_identity_env`
actually reads (`secret_ref.dir`) — reintroducing this exact bug for that
case. `GetIdentityAccountCommand` (step 2 above) is a pure read of the
account's real stored `secret_ref`, with no reconstruction risk.

~~1. In `useAgentControllerStatus.ts` (or wherever the launch flow first has
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
   `ListAgentIdentitiesCommand` twice per mount.~~

This changes zero *new* backend RPC surface — `GetIdentityAccountCommand` and
`ListAgentIdentitiesCommand` both already existed and were already exercised
elsewhere. It does touch existing backend logic (`inject_identity_env`'s
gate + probe canonicalization, per point 5 above), which the original plan
didn't anticipate needing.

## 5. Test plan (as implemented)

1. `launch-flow.test.ts`: linked-account overrides the generic dir via
   `GetIdentityAccountCommand`; falls back to generic on no-link, non-oauth
   `secret_ref`, or a thrown lookup; matches a legacy-alias-only link;
   prefers the last match when both a canonical and alias row exist; never
   invokes the lookup at all for an api-key provider (`authType !==
   "oauth"`), even one that also sets `authConfigDirEnvVar` for an unrelated
   reason (Kimi).
2. `provider-id-aliases.test.ts`: direct unit coverage for
   `canonicalProviderId`/`lastLinkedAccountId`, plus a drift-guard test
   (same idiom as `pin-consistency.test.ts`) keeping the frontend alias
   table in sync with `agentmux-srv/src/backend/providers.rs`'s `ALIASES`.
3. `useAgentControllerStatus.test.ts`: `relogin()` passes the alias-linked
   `account_id` through as `existingAccountId`, not `undefined`.
4. `inject.rs`: `inject_oauth_class_succeeds_when_the_only_binding_is_under_a_legacy_alias`
   and `inject_oauth_class_probe_canonicalizes_a_legacy_alias_binding` cover
   the two backend gaps from point 5 above.
5. Live `task dev` (outstanding — see the tracking PR): open an agent with
   an already-authenticated, explicitly bound account — confirm the pane
   goes straight to ready, no "Log in" prompt; a genuinely first-time agent
   still shows "Log in" as before.

## 6. Open question for go-ahead — resolved

§4's fix is implemented (PR #2377, still awaiting the live `task dev` check
in §5 item 5 before merge). The second, independent question — whether to
open a tracking discussion for the broader auth-architecture consolidation
(analogous to #2375), so a future bug in this family is tracked in one place
instead of re-diagnosed — remains open and unrelated to merging this fix.
Reference issue #678 for that broader "one Credential Broker" work in the
meantime (per direct user feedback: append there rather than opening a new
discussion, since #678 already exists for the identity-system domain).

---

*Diagnosed, implemented, and code-reviewed (PR #2377) — awaiting a live
`task dev` check before merge.*
