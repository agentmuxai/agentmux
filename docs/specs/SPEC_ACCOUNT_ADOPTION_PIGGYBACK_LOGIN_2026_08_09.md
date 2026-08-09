# SPEC — Account adoption: piggyback an unlinked agent onto an existing login

**Date:** 2026-08-09
**Status:** Proposed — Surface A superseded by
`SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md` (user-preferred
direction: bind from the Armory, not from the failure row). Surface B
(Identity-tab "Link existing account…") remains proposed as the
agent-side complement; see that spec's §6.
**Trigger:** Live v0.54.14 testing on a second machine (claudius). A
pre-existing legacy agent ("Agent1", blank `identity_id`, no direct link)
was correctly refused by the layer-3 spawn gate (#2463/#2464) and — after
#2482 — now shows the real message: *"no credentials for claude … Bind an
account for this provider in the Armory."* But the same machine already had
a **valid, bound Claude account** (used successfully by a different agent
minutes earlier). The user's only offered recovery paths were a fresh OAuth
or seeding from the global CLI login — there is no way to say "just use
that account over there."

---

## 1. What already exists (do not rebuild)

| Surface | Mechanism | Piggybacks on |
|---|---|---|
| Launch modal account picker | `AgentLaunchModal` — lists all accounts for the agent's provider, auto-picks the first when none selected | Any Armory account (new launches only) |
| "Use existing login" recovery action | `useAgentControllerStatus.useGlobalLogin()` — seeds the **global CLI login file** (`~/.claude`) into a freshly-minted IdentityAccount + isolated dir, links it | The OS-level ambient login |
| "Login Again" recovery action | `relogin()` — fresh OAuth; reuses the agent's **own** previously-linked account id (`existingAccountIdFor`) so retries don't mint orphans | The agent's own prior account |

**The gap:** an agent with *no* link (legacy row, post-account-delete
cascade, fresh isolated channel) cannot adopt an *existing Armory account*
that some other agent already uses. All the machinery below the UI already
supports it — `db_agent_identity_links` is many-links-to-one-account by
design, `LinkAgentIdentityCommand` upserts with
`ON CONFLICT(agent_id, provider) DO UPDATE`, and the spawn resolver
injects whatever the link points at. This is a UI/flow feature, not a
backend feature.

## 2. Design

### 2.1 Surface A — the Auth-classified failure row (primary)

When the failure row shows the `FailureClass::Auth` "No account linked"
state, add one recovery action alongside "Log in" / "Use existing login":

- **"Use existing account"** — enabled when ≥1 *candidate account* exists.
  - Exactly one candidate → the button is labeled with it
    ("Use account: *work-claude*") and one click adopts it.
  - Multiple candidates → the button opens a small inline picker
    (name + status dot + which agents already use it), one click adopts.

**Adopt =** (all existing primitives, mirroring `useGlobalLogin`'s
post-registration steps, which this shares code with):
1. `LinkAgentIdentityCommand { agent_id, account_id, provider }` (upsert).
2. Update block `cmd:env` with the provider's config-dir env var pointing
   at the adopted account's `OAuthConfigDir` dir — same live-refresh step
   `useGlobalLogin` already performs so the running persistent agent picks
   the credential up on its next message without a restart.
3. Wrap in `beginRecoveryFlow()/endRecoveryFlow()` exactly like the other
   recovery actions (the fast-fail send guard and concurrent-recovery
   rules from PR #2338 apply identically).
4. Retry the failed turn (same post-login retry the other actions use).

### 2.2 Surface B — the per-agent Identity tab

`agent-identity-links-panel` today offers "Connect \<Provider> account"
(fresh login). Add **"Link existing account…"** beside it with the same
candidate picker → `LinkAgentIdentityCommand`. No retry semantics needed
here (not in a failure context); the next spawn resolves the link.

### 2.3 Candidate set (both surfaces)

From the shared account cache (`loadAccounts()` — live as of #2474):
- `resolve_provider_alias(account.provider) == resolve_provider_alias(agent.provider)`
  (canonicalize BOTH sides — the alias-mismatch class codex flagged on
  PR #2377 applies here verbatim).
- OAuth-class accounts only (`secret_ref.backend == "oauth_config_dir"`).
  Api-key accounts don't gate spawns and already inject per-link.
- Sort: `status == "valid"` first, then most-recently-updated. Show the
  status dot — an expired account is selectable (the spawn-time probe
  refreshes status, and adopting-then-relogging is still fewer steps than
  a from-scratch OAuth) but visibly marked.
- Exclude the agent's already-linked account id (nothing to adopt).

## 3. Explicit non-goals / rejected alternatives

- **Silent auto-adoption** (gate finds no link but exactly one valid
  account exists → auto-link and proceed). Rejected as default behavior:
  an agent silently starting to act under an account the user never chose
  is exactly the attributability failure the "single point, not global"
  invariant (PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md §7) exists
  to prevent — one personal-vs-work account mixup pays for all the clicks
  this would ever save. Could ship later behind an explicit opt-in setting
  if single-account users want it; the candidate-set + adopt code from
  §2 is reusable as-is.
- **Cross-channel / cross-machine piggyback** (claudius adopting the main
  box's login). Accounts live per-channel in `db_accounts`; sharing across
  channels or machines is a credential-sync feature (muxbus-transported,
  keychain-backed) with a real threat model — separate spec if wanted.
- **Backend changes.** None needed. The gate's refusal already carries the
  provider id; the frontend has the accounts. (Optional nicety — the gate
  could count candidates and phrase its message "2 existing accounts could
  be linked" — deferred; the frontend can compute the same thing.)

## 4. Sharing semantics (already true today, now more visible)

Multiple agents linking one account means shared `OAuthConfigDir` and
shared rate limits/token refresh. This is the existing, shipped behavior
for accounts picked in the launch modal — adoption adds no new mechanism,
but the picker showing "used by AgentX, AgentY" makes the sharing legible
at decision time.

## 5. Test plan

- Unit: candidate-set filter (alias canonicalization both sides,
  oauth-class-only, exclusion of the already-linked id, status sort).
- Unit: adopt action wiring — link RPC called with canonical provider,
  `cmd:env` updated with the adopted dir, recovery-flow counter
  begun/ended, retry fired (mirror the existing `useGlobalLogin` tests in
  `useAgentControllerStatus.test.ts`).
- Live check: the exact claudius repro — legacy agent + one existing valid
  claude account → one click on "Use account: …" → next message runs.

## 6. Open decision for review

Surface A's placement assumes the failure row can grow a third action
without crowding (it already holds Log in / Use existing login / Login via
terminal in some states). If it's too dense, the fallback is Surface B
only + the failure message deep-linking to the Identity tab — one more
click, zero new failure-row complexity.
