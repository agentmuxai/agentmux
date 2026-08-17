# SPEC: muxbus free-account abuse hardening — closing the sign-up/messaging backdoors

**Date:** 2026-08-17
**Status:** Partially implemented — see §10. Gaps 2 (partial) and 3 shipped; gap 1's fix and gap 2's enforcement flip deliberately deferred pending real evidence.
**Author:** AgentX
**Repos touched:** `agentmux-cloud` (all implementation), `agentmux` (this doc; status-bar network panel UI is unaffected — no changes needed there)
**Related:** `SPEC_MUXBUS_OWNER_GATE_AND_COST_CAP_2026_08_11.md` (the quota system this spec builds on — §6 of that doc already flagged gap #1 below as its own explicit prerequisite, never closed), `SPEC_FREE_TIER_PRICING_2026_06_21.md` (free-tier limits, implemented), `SPEC_MUXBUS_MULTI_TENANT_SECURITY_2026_07_06.md` (Phase 1, partially landed), `PLAN_PER_AGENT_CREDENTIAL_BINDING_2026_07_06.md` (documents gap #2 below as never-deployed), `SPEC_JEKT_LAN_WAN_TRUST_HARDENING_2026_08_13.md` §5.1 (the account-scoped M2M client *design* — that spec's own status header still marks this "proposed/not implemented" as of 2026-08-13/14; §2 below cites the actual running code directly rather than relying on that header, since the two disagree — see §2's footnote)

---

## 1. Motivation and scope

Growing user count means the "Connect with AgentMux" flow on the status-bar
network panel — Google OAuth via Cognito, PKCE, cloud-relayed callback
(`SPEC_MUXBUS_CLOUD_RELAYED_LOGIN_CALLBACK_2026_08_15.md`) — is now a
realistic attack surface, not just a trusted-insiders convenience. This
spec asks two questions and answers both from the actual deployed code,
not assumptions:

1. **Is a new free signup's message volume actually capped?** Yes — see
   §2. This part is already solid.
2. **Can a new signup find a backdoor that bypasses that cap, or
   impersonate another agent?** Yes, in two concrete, already-documented-
   but-never-closed ways — see §3 and §4. Those are this spec's real
   subject.

This is deliberately narrow: it does not propose new features, only
closing gaps that already have a named owner and a stale "not done yet" in
existing specs. Where a gap maps to an OWASP API Security Top 10 (2023)
category or a documented AWS Cognito abuse-prevention pattern, it's cited
— this is a hardening pass grounded in established practice, not novel
design.

## 2. What's already solid — the free-tier cost cap (confirmed implemented)

`SPEC_MUXBUS_OWNER_GATE_AND_COST_CAP_2026_08_11.md`'s quota design is live
in the running code (its own header still says "Draft" — a stale label,
worth fixing in that doc separately). `consumeQuota()`
(`muxbus/server/src/quota.ts:95-169`) does an atomic DynamoDB increment
keyed on `(accountUserId, "YYYY-MM#resource")`, capped at `FREE_TIER.jekt_messages
= 2,000/month` (`quota.ts:12`) for `billingTier === 'free'`, and is wired
into every real send path: `POST /api/messages`, `POST /reactive/inject`,
MCP `send_message`/`broadcast_message`, and `POST /agents/provision`.

Two things worth calling out as already correctly hardened, since a naive
design would get them wrong:

- **Keyed on the human account, not the agent client or API key.**
  `auth.ts:111-122` resolves `accountUserId` from the Cognito account that
  *owns* the M2M client, specifically so one signup can't multiply its
  free tier by provisioning more agent identities. This reflects the
  account-scoped Cognito M2M client design `SPEC_JEKT_LAN_WAN_TRUST_HARDENING_2026_08_13.md`
  §5.1 proposed — confirmed live in the actual running code (not the
  cited spec's own status label, which as of that doc's last edit still
  read "proposed/not implemented" for this specific item; the code has
  since caught up without the doc being updated, the same stale-header
  pattern noted for `SPEC_MUXBUS_OWNER_GATE_AND_COST_CAP_2026_08_11.md`
  above). Direct evidence: `agent-ownership.ts`'s own module doc comment
  states the redesign in the present tense ("one Cognito M2M client now
  covers every agent_id an account runs"), the whole ownership-table
  mechanism (`agent-ownership.ts`, `agent-binding.ts`) only makes sense
  built on top of that model, and — strongest signal — live CloudWatch
  logs for `/aws/lambda/muxbus-server` show `isAgentOwnedByAccount()`
  actively querying real production traffic against this exact table
  right now (see §10's account `4478f488-...` trace).
- **`billingTier` is a server-derived JWT claim, never client input.** The
  Pre-Token Generation Lambda (`cognito/pre-token.ts:59-102`) compares
  Cognito's own verified `email` against a single hardcoded owner email
  and mints `billing_tier` into the token itself — a fresh Google signup
  gets `'free'` by construction, with no self-service path to anything
  else (no Stripe webhook grants tier; `reportToStripeMeter` only emits
  usage). A new signup cannot escalate itself.

Net: the cap is real, correctly scoped, and not self-escalatable. Do not
touch this system — the two gaps below are about what happens *around*
it, not weaknesses in it.

## 3. Gap 1 (P0): the legacy shared-token bypass is unconditionally unmetered

**Confirmed live**, not theoretical: `auth.ts:12` — `DISABLE_LEGACY_AUTH =
process.env.DISABLE_LEGACY_AUTH === 'true'`. `muxbus-stack.ts`'s deployed
Lambda environment block never sets this variable, so it defaults to
`false` in production. Any request bearing the shared `muxbus-api-key`
secret (`auth.ts:86-96`) authenticates via `{mode: 'legacy'}`, and
`getBillingTier()` maps that unconditionally to `'metered'` (`auth.ts:204`)
— `consumeQuota()`'s `metered` branch (`quota.ts:152-153`) always returns
`allowed: true`, no volume check of any kind, ever.

This is **not** a new-signup vector specifically (it requires possessing
the shared secret, not just an account), but it's the single largest hole
in the whole cost-cap story precisely because it has no ceiling at all —
`SPEC_MUXBUS_OWNER_GATE_AND_COST_CAP_2026_08_11.md` §6 already named this
"the single biggest hole, bigger than the sign-up gap" and listed closing
it as a **prerequisite**. It's still open a week later. This maps to
OWASP **API4:2023 Unrestricted Resource Consumption** — a missing
consumption limit on an authenticated path is exactly that category's
definition, and OWASP's own example of an unbounded-cost incident (a
$13→$8,000 monthly bill from one missing cap) is the realistic failure
mode here if the legacy key ever leaks or is reused past its intended
lifetime.

**Fix:** set `DISABLE_LEGACY_AUTH=true` in `muxbus-stack.ts`'s Lambda
environment once every current legitimate legacy-token holder is
confirmed migrated to Cognito M2M (`COGNITO_DEPLOY_PREREQS.md:58`
documents the token as intended-temporary already — this spec just closes
it). If any caller still needs it, treat that as a signal to finish their
migration, not a reason to leave the bypass open indefinitely.

## 4. Gap 2 (P0): agent-identity spoofing is unenforced on every send path

**Confirmed live**: `POST /api/messages` (`index.ts:270-326`) and the MCP
`send_message`/`broadcast_message` tools (`index.ts:692-748`) all derive
the *sending* agent's identity purely from a client-supplied `X-Agent-ID`
header/argument, with **zero** `checkAgentBinding()` call anywhere near
them — verified directly by reading both handlers. Compare to
`/reactive/inject`, `/reactive/pending`, `/reactive/ack`, `/reactive/release`,
and `/reactive/status`, which **do** call `checkAgentBinding()`
(`index.ts:386, 480, 514, 542, 579`).

But even that partial coverage doesn't currently enforce anything:
`ENFORCE_AGENT_BINDING` is never set in `muxbus-stack.ts`'s deployed
environment either (same pattern as gap 1), and
`PLAN_PER_AGENT_CREDENTIAL_BINDING_2026_07_06.md:8` already documents this
as log-only, never enforced. So today, on every message-send path without
exception, any authenticated account (a brand-new free signup included)
can claim to be sending as *any* `agent_id` string — a genuine identity
spoof, not just a quota question. This is OWASP **API2:2023 Broken
Authentication** territory: the system authenticates the *account* but
never authenticates the *claimed sender identity* riding on top of it,
and nothing downstream currently corrects for that at the muxbus layer
(the jekt `TRUST=` marker system in `agentmux-srv` is a receiving-side
mitigation for exactly this gap, not a substitute for closing it at the
source — see §6).

**Fix, two parts:**
1. Add `checkAgentBinding()` to `POST /api/messages`, `send_message`, and
   `broadcast_message` — matching the pattern already used on every
   `/reactive/*` route.
2. Set `ENFORCE_AGENT_BINDING=true` in `muxbus-stack.ts`'s deployed
   environment once (1) is done and existing traffic is confirmed
   compliant via the log-only signal already being collected — turning on
   enforcement before adding the missing checks would just produce a
   different set of unenforced routes.

## 5. Gap 3 (P1): instant, unverified account provisioning on the sign-up path itself

`selfSignUpEnabled: false` (`muxbus-cognito.ts:165`, "Invitation-only")
only gates Cognito's *native* username/password `SignUp` API. It does
**not** gate federated Google sign-in — confirmed by reading the pool
config directly. A first-time Google OAuth completion instantly
provisions a working Cognito user (email auto-verified via Google,
`autoVerify: { email: true }`, `muxbus-cognito.ts:167`) with no
invitation, approval, or waiting step. No MFA is configured anywhere in
the pool. No CAPTCHA, WAF Bot Control, or Fraud Detector integration
exists at any layer.

This is not a cap-bypass (§2's per-account quota still applies to each
new account individually) — it's a **volume-of-accounts** problem: bulk
account creation is bounded only by Google's own signup friction, and
each new account is a fresh 2,000-message/month allowance plus whatever
`agent_provisions` (50/month) and other free-tier resources it can claim.
At the scale of "many free accounts scripted in parallel," the effective
aggregate cost ceiling is `(free-tier limits) × (however many Google
accounts an attacker is willing to create)`, which is not actually a
ceiling. This is squarely the AWS-documented "sign-up fraud" problem
Cognito's own guidance addresses directly (AWS Security Blog, "Reduce
risks of user sign-up fraud... with Amazon Cognito user pools"; AWS WAF
Fraud Control ACFP is built for exactly this).

**Fix options, cheapest to most involved — pick based on actual observed
signup volume, don't over-build ahead of real abuse:**
1. **Pre sign-up Lambda trigger + CAPTCHA on the desktop-side flow** — the
   status-bar panel is a native app opening a browser, not a web signup
   form, so a classic CAPTCHA widget doesn't fit cleanly; a lighter
   version (rate-limit new-account creation per source IP at the
   `desktop-callback`/login-relay layer, which already has an IP-keyed
   `RateBucket`, just not currently applied to *new-account* creation
   specifically) is a smaller lift.
2. **AWS WAF Bot Control** in front of the Cognito hosted UI /
   `desktop-callback` — "Common" tier is a low-effort managed-rule
   addition; catches unsophisticated scripted signups without any app
   code changes.
3. **AWS WAF Fraud Control ACFP** — purpose-built for account-creation
   fraud (reputation/risk-based automated vetting, disposable-email
   detection), the most complete option, also the most infrastructure to
   add and operate.
4. Note the login-relay's existing `RateBucket` (`login-relay.ts:52-75`)
   is documented as in-memory and Lambda-instance-local — a distributed
   signup script fanning out across cold starts is bounded by "Lambda
   concurrency × limit," not the limit alone. Any fix here should either
   move to a shared store (DynamoDB/Redis-backed) or be paired with an
   edge-layer control (WAF) that doesn't have this weakness.

Not proposing a specific option here — this is a judgment call on
expected abuse volume vs. implementation cost, not something to guess at
silently. Recommend starting with option 1 (cheapest, closes the
worst-case unbounded-fan-out gap) and adding WAF Bot Control only if
observed signup patterns justify it.

## 6. Gap 4 (P2, lower priority — already flagged elsewhere, noted for completeness)

`broadcastToApiGatewayConnections` (`broadcast.ts:39-91`) does a full
DynamoDB table `Scan` plus per-connection push fan-out on *every* message
— cost that scales with total connected users, not attributable to the
sender, and not currently capped or billed to whoever triggered it.
Already flagged as unsolved in `SPEC_MUXBUS_OWNER_GATE_AND_COST_CAP_2026_08_11.md`
§7. Not re-litigated here in detail; included because a free-account
abuse review would be incomplete without noting that a single free
account sending broadcast messages already has an outsized, unmetered
cost footprint relative to its own quota consumption (one `jekt_messages`
unit charged, but O(N) AWS calls incurred). Worth a follow-up spec of its
own rather than folding into this one.

## 7. Relationship to the jekt `TRUST=` marker system

Worth being explicit about scope: the receiving-side trust-tier system
(`TRUST=host-verified/self-declared/network-claimed`, reagent's
`SIG=verified` WAN signing, per-agent LAN signing) already assumes an
unauthenticated/spoofable sender is the **normal, expected** case for
network-tier traffic — that's why `TRUST=network-claimed` exists and
`TIER=sensitive` isn't forced by trust alone. Closing gap 2 (§4) doesn't
make that system redundant; it removes one specific, unnecessary source
of spoofing (an authenticated muxbus account lying about which agent it
is) without changing the fact that cross-machine identity is fundamentally
unproven by design elsewhere in the protocol. Don't read this spec as
proposing to tighten `TRUST=` tier rules — that's a separate, already
carefully-scoped system (see `CLAUDE.md`'s jekt security rules section)
and out of scope here.

## 8. Recommended rollout order

1. **Gap 1** (§3) — flip `DISABLE_LEGACY_AUTH=true`. Smallest change,
   closes the largest unbounded-cost hole, already named a prerequisite
   in an existing spec. Do this first and independently of the rest.
2. **Gap 2** (§4) — add the three missing `checkAgentBinding()` calls,
   confirm via logs that no legitimate traffic would be rejected, then
   flip `ENFORCE_AGENT_BINDING=true`. Two-step by design — enabling
   enforcement before adding the missing checks would just move the gap.
3. **Gap 3** (§5) — pick a fix tier based on observed signup abuse once
   1 and 2 are live; not urgent relative to the other two since it's a
   volume multiplier on an otherwise-capped-per-account system, not an
   unbounded hole by itself.
4. **Gap 4** (§6) — separate follow-up spec, not blocking on this one.

## 9. Out of scope for this pass

- MFA on the Cognito pool — a real hardening option AWS's own guidance
  recommends, but changes user-facing login UX (extra step on every
  sign-in) rather than closing a silent backdoor; a product decision, not
  a pure security fix, deliberately not bundled in here.
- Amazon Fraud Detector ML-based risk scoring — the heaviest-weight option
  from §5's list; not recommended until cheaper options are shown
  insufficient by real abuse data.
- Retroactively metering/capping the legacy `'metered'` tier's own volume
  (as opposed to disabling it outright) — §3's fix is to close the
  bypass, not to give it a cap; if legitimate metered use needs to
  continue post-cutover, that's a Stripe-integration question out of this
  spec's scope.

## 10. Implementation notes (added post-implementation)

What shipped in `agentmux-cloud` (server package `1.6.0` → `1.7.0`), and —
more importantly — what was deliberately **not** flipped on despite being
named in §8's rollout order, with the real evidence that changed the plan:

**Shipped, unconditionally safe (all purely additive):**
- `checkAgentBinding()` added to `POST /api/messages`, MCP `send_message`,
  and MCP `broadcast_message` — closes §4's "zero routes check this"
  finding. Still log-only (matches the other five routes) since
  `ENFORCE_AGENT_BINDING` isn't set — see below for why not yet.
- Legacy-auth-mode logging added to `auth.ts` — before this change,
  **zero code anywhere logged when the legacy shared-token path was
  used**, which is *why* §3's rollout step 1 ("confirm via logs") wasn't
  actually possible before this PR. This is a prerequisite for gap 1's
  fix, not the fix itself.
- Gap 3's fix, scoped down from §5's original menu: moved the
  login-relay write-path rate limiter (`/api/login-relay`,
  `/api/login-relay/submit`) from the in-memory, Lambda-instance-local
  `RateBucket` to a durable, cross-instance DynamoDB-backed limiter
  (`LoginRelayStore.checkDurableRateLimit`, reusing the existing
  login-relay table — no new infrastructure). This closes the
  specific "distributed attacker fanning out across cold starts" gap
  §5 flagged, for the write path specifically. **Full CAPTCHA / Cognito
  pre-sign-up Lambda trigger (§5's options 2–3) were investigated and
  intentionally not attempted in this pass** — confirmed no pre-sign-up
  trigger exists on the User Pool today (only pre-token-generation),
  and Google-federated sign-in doesn't route through any of our own
  code until after the account already exists, so a CAPTCHA gate would
  require either a net-new pre-sign-up Lambda + a client-side CAPTCHA
  integration point in the desktop app's login flow, or Cognito Managed
  Login's newer CAPTCHA support — real design work, not a same-pass
  addition alongside gaps 1/2's fixes. Left as a follow-up.

**Deliberately NOT flipped on, despite §8 listing them as this pass's
goal — real production evidence overrode the plan:**

- **`DISABLE_LEGACY_AUTH=true`** — not set. With the new logging just
  shipped, there's now a way to observe usage going forward, but zero
  historical signal existed to check *before* this PR (the whole
  point of shipping the logging first). Flipping this blind, with a
  bypass that's unconditionally unmetered, risks silently breaking
  whatever currently holds that shared key. Follow-up: check the new
  log line's volume after this deploys for a real observation window,
  then flip once it reads zero (or once every remaining caller is
  confirmed migrated).
- **`ENFORCE_AGENT_BINDING=true`** — not set, and this isn't a "no
  evidence yet" gap like the one above, it's a **positive, confirmed
  reason not to**: `CloudWatch` logs for `/aws/lambda/muxbus-server`
  show one real account (`4478f488-2031-70d3-f5d6-e162ce562efe`)
  actively polling `GET /reactive/pending` for three agent IDs
  (`clare`, `agento`, `masty`) with no ownership row, hundreds of times
  over several hours, at the exact moment this spec was being
  implemented — not a one-off blip. Flipping enforcement right now
  would immediately 403 that account's real, currently-working traffic.
  Root cause, traced through `agent_credentials.rs`: per-agent
  credential provisioning (`ensure_agent_credential` → `POST
  /agents/provision`, which is what populates the ownership table) is
  only called by AgentMux client versions that include it: an older
  client never attempts provisioning at all and falls back to the
  shared account-level token with a self-declared `X-Agent-ID` — the
  exact pre-migration behavior `ENFORCE_AGENT_BINDING` is meant to
  retire. **This means enforcement has a real client-version-adoption
  dependency that §8 didn't account for** — it can't safely turn on
  until enough of the active user base is running a client new enough
  to provision automatically, which is a rollout/telemetry question,
  not a one-PR code change. `backfillOwnershipFromLegacyRegistry()`
  (in `agent-ownership.ts`, a standalone script, not wired into any
  deploy path) exists for accounts that provisioned under the *old*
  per-(account,agent) Cognito-client scheme and would fix a different
  gap than this one — not confirmed to apply to this specific account,
  and out of scope to run speculatively.

Net: this pass closes the parts of gaps 2 and 3 that were safe to close
without breaking real traffic, and turns gap 1 from "unverifiable" into
"observable" — but the two actual kill-switch flips §8 named as the
goal remain open, now for a documented, evidence-based reason rather
than an oversight. Follow-up spec/PR once the observation window (gap 1)
and client-version-adoption data (gap 2) exist.
