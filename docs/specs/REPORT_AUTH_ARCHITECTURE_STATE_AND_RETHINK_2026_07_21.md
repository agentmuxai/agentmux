# AgentMux Auth Architecture — Current State & Rethink

**Date:** 2026-07-21
**Scope:** Every OAuth/credential system in AgentMux — provider-CLI login, MuxBus/AgentMux Cloud, and the
Armory service-account OAuth scaffold — as they exist today, plus external research into how comparable
tools solve the same problems, plus a concrete target architecture.

**Ground truth basis:** the codebase as checked out on `agenta/release-v0.54.1` (matches what's actually
running live today). PR #2255 (`agenta/login-single-point-enforcement-clean`, **open, unmerged** as of
this writing) reworks part of this system; its changes are described separately in §3 and never blended
into the "current state" sections, since it isn't real yet.

> **Staleness note (2026-08-03):** PR #2255 has since merged (`95d2cfe7`, "single-point login enforcement for oauth providers"). Every "open, unmerged — not real yet" qualifier below (§3 heading, §4 intro, and inline mentions) is now describing shipped, current behavior, not a proposal — re-read §3 as "what changed" rather than "what would change." Table/column names throughout may also predate the `db_identity_accounts` → `db_accounts` rename; cross-check against current code before relying on schema specifics.

**Implementation status (updated same day):** Phases A–C of §6's target architecture shipped as PRs
#2260, #2262, #2263. Phase D (the device-flow shim) was **not built** — a dedicated feasibility spike
found it's not viable for any of the three target providers (Anthropic, OpenAI, Gemini). See §8 for the
spike's findings and the resulting scope decision.

---

## 0. Executive summary

AgentMux has **three separate, independently-implemented OAuth/credential systems** sharing nothing but a
SQLite connection handle:

1. **Provider-CLI identity** (`db_accounts`/`SecretRef`/`db_agent_identity_links`) — lets a spawned Claude
   Code/Codex/OpenClaw CLI process authenticate as a specific bound account.
2. **MuxBus / AgentMux Cloud** (`db_muxbus_credentials`) — a Cognito PKCE login for the *human user's*
   cloud account, powering a persistent WebSocket subscription. Plaintext tokens, no keychain, one global
   row app-wide.
3. **Armory service-account connections** (`identity/oauth_client.rs`) — a third OAuth implementation for
   "connect a Google/Microsoft/Slack/GitHub account" buttons, currently a scaffold with no real client IDs
   provisioned.

Within system 1 alone, **five independent code paths** can trigger a provider login today, four of which
duplicate the same "spawn CLI, scrape URL, poll for success" logic with small, drifted variations. PR
#2255 (open) consolidates four of those five behind one orchestrator, but even if it merges as written,
the underlying *mechanism* — spawn a real visible OS terminal window so the CLI's own browser-opening code
has a console to attach to — stays a platform-specific, fragile workaround, not a fix for the root cause.

External research (§5) converges cleanly on what a durable fix looks like: **the visible-terminal
workaround is the same category of hack every comparable tool has landed on in the absence of the OAuth
2.0 Device Authorization Grant (RFC 8628)** — a flow purpose-built for exactly this "no browser/console
available to the process doing the login" situation, already the default or standard option in `gh`,
`docker login`, `az login`, `aws sso login`. None of AgentMux's three target CLIs expose it reliably today,
but that's a reason to build AgentMux's own device-flow-shaped shim, not a reason to keep patching the
terminal-spawn approach.

The recommended target architecture (§6) is a single backend-owned **Credential Broker** — one component
that owns issuance, OS-keychain-backed storage, proactive+single-flight-guarded refresh, and per-session
credential isolation for *all three* of today's separate systems — paired with a provider-agnostic
device-flow login shim that replaces the terminal-spawn workaround outright. This closes the
fragmentation problem (one broker, not five call sites), the credential-isolation problem (structural,
not just a UI toggle), and heads off a concrete failure mode AgentMux hasn't hit yet but Claude Code
itself has (concurrent-refresh races, Claude Code issues #25609/#29896) before it does.

---

## 1. Current state — every login-triggering code path

Five distinct, UI-reachable entry points exist today; only two share an implementation.

| # | Trigger | Handler | Shares code with | Host/backend primitive |
|---|---|---|---|---|
| 1a | `/login` slash command | `commands/global/login.ts` → `forceProviderLogin` | 1b (`force-login.ts`) | `run_cli_login` (piped/PTY) |
| 1b | Failure-banner "Login Again" | `useAgentControllerStatus.ts`'s `relogin` → `forceProviderLogin` | 1a | `run_cli_login` (piped/PTY) |
| 1c | Gated launch flow / "Retry Login" (runs on **every** unauthenticated pane mount) | `launch-flow.ts` Phase 2 — hand-rolled URL-open + poll | none | `run_cli_login` (piped/PTY), called directly |
| 1d | New Agent modal "Connect"/"Reconnect" | `PreLaunchAuthPanel.tsx` / `AuthFlowController` state machine | none | **entirely different RPC surface**: `agentmux-srv`'s `auth.start`/`auth.poll` — not the CEF host at all |
| 1e | "Use existing login" / "Login via terminal" (pane failure banner) | `useAgentControllerStatus.ts`'s `useGlobalLogin`/`loginViaTerminal` | none (two more independent implementations) | `seed_provider_auth_from_global` / `open_login_terminal` |

Both underlying CEF-host primitives exist for the same reason: `run_cli_login`/`run_cli_login_pty` spawn
the CLI piped/PTY (no attached console) and scrape stdout for a login URL — this is fast when it works,
but **Claude Code v2.1.x's OAuth flow opens the browser itself from inside its own process**, which
requires a real attached console to do. When it doesn't (the common case), nothing happens and the tier
silently times out after 15s–5min depending on the path. `open_login_terminal` is the actual fix (spawns a
**visible** OS console — `CREATE_NEW_CONSOLE` on Windows — so the CLI's in-process browser-open call has
something to attach to), but today it's wired to only two of the five paths (1e), Windows-only, and
undiscoverable unless a user knows to look for "Login via terminal" specifically.

Path 1d is the worst-off: it goes through a completely separate `agentmux-srv` RPC namespace
(`auth.start`/`auth.poll`, backed by `spawn_auth_cli`/`spawn_auth_cli_pty` in
`identity_handlers.rs:322-971`) that has **no visible-console fallback at all** — grepping that file for
`CREATE_NEW_CONSOLE`/`open_login_terminal` returns zero hits. For Claude Code today, clicking "Connect" in
the New Agent modal is a structural dead end.

**Confirmed-dead code**, independent of PR #2255's own audit: `clearProviderAuth`/`getProviderAuthStatus`/
`checkCliAuthStatus` and `openClaudeCodeAuth`/`getClaudeCodeAuth`/`disconnectClaudeCode` are fully wired
end-to-end (CEF IPC → host command → frontend wrapper) but have **zero call sites anywhere in the UI**.

---

## 2. Current state — data model and bindings

### 2.1 `IdentityAccount` / `SecretRef` / `db_accounts`

One row per bound credential. `SecretRef` is a tagged enum with five variants: `Env`, `SecretsManager`
(unimplemented — Phase 3, `resolve_secret` returns `SecretsManagerUnsupported`), `PlaintextDev`
(debug-build-only), `OAuthConfigDir { dir }` (points the provider CLI at an isolated directory rather than
holding a secret value at all — AgentMux never stores the actual OAuth token for CLI providers, it only
ever points the CLI at a directory and lets the CLI manage its own token file), and `Keychain { service,
account }` (backs the Armory API-key flow and the service-account OAuth scaffold, via a real `keyring`-crate
wrapper — `identity/secret_store.rs`).

### 2.2 `db_agent_identity_links`

Links an **agent definition** (not instance) to at most one account per provider — `PRIMARY KEY (agent_id,
provider)`. Deletion is an explicit application-level transaction (deletes links before the account row),
deliberately not trusting the DDL `ON DELETE CASCADE`, because legacy databases that arrived via
`ALTER TABLE … RENAME` never got the FK clause retrofitted — a real production gap the code compensates
for explicitly rather than assuming schema correctness.

### 2.3 `use_ambient_login` (per-agent-definition flag)

`i64`, defaults to `0` ("fail closed"). When `1`, the spawn gate (§2.4) skips injecting a config-dir env
var entirely and lets the CLI fall through to whatever global login already exists on the machine — logged
via `identity.spawn.ambient:`, never silent. Pre-existing agents were grandfathered to `1` by a migration
(`m0017`) **unless** they already had an oauth-class direct link. Default for new agents is `0`.

### 2.4 The spawn-time enforcement gate — `identity/resolver.rs`

The authoritative decision point, run at every CLI spawn via `inject_identity_env_with_broker`. Simplified:

- No launch-modal instance record for this pane → no gate at all (quick-launch panes are outside the
  managed-credentials contract entirely).
- For each oauth-class binding (`claude`/`codex`/`openclaw` — narrower than the 5 providers the *frontend*
  labels `authType: "oauth"`; see §2.5): missing account, lookup error, or a non-`OAuthConfigDir` secret
  ref all route through `gate_oauth_failure`, which **honors `use_ambient_login`** — `1` → proceed with no
  isolation, `0` → refuse the spawn outright (`SpawnGateError::MissingCredentials`, process never created).
- A second, structurally identical gate fires if the agent *definition's own* provider field is oauth-class
  and has **no binding at all** (never linked, or a link was cascaded away by a delete) — this closes a
  real regression a 2026-07-08 refactor introduced, per a dedicated regression-canary test that explicitly
  warns future readers not to "fix" it without reading that history first.
- A successful resolution also runs a cheap on-disk expiry probe (reads `.credentials.json` directly) and
  republishes `identityaccounts:changed` if status changed — the only place in the provider-CLI system that
  proactively checks freshness at all, and only reactively, at spawn time, not on a timer.

### 2.5 Provider registry — two parallel tables, one narrower gate

The frontend (`providers/index.ts`) lists 9 providers, 5 marked `authType: "oauth"` (claude, codex,
gemini, openclaw, copilot). The Rust spawn gate (`resolver::provider_class`) only actually treats **3**
of those (`claude`/`codex`/`openclaw`) as oauth-class for isolation-dir minting and gating purposes —
**gemini and copilot are oauth-typed in the UI but not subject to any of the isolation or enforcement
machinery described above.** Whether that's intentional (those two don't need per-account isolation yet)
or drift wasn't determined by this investigation and is worth an explicit decision, not an assumption.

The frontend and Rust provider tables are independently maintained with a "keep in sync" comment that has
already drifted in one place (Claude's launch args differ between the two). A third, much older, 3-provider
hardcoded list also exists in `agentmux-cef/src/commands/providers.rs` for legacy install-info commands.

---

## 3. What PR #2255 changes (open, unmerged — not real yet)

Three retro/plan docs (only present on the PR branch) establish the fix incrementally:

1. **`retro-headless-login-browser-open-2026-07-20.md`** — diagnoses the headless-spawn root cause and
   introduces `runProviderLogin`, a 3-tier orchestrator (capture URL → Claude-only seed-from-global-login
   → open a real terminal + poll), wired into `/login` and "Login Again"; ports `open_login_terminal` to
   macOS/Linux.
2. **`retro-login-three-code-paths-2026-07-20.md`** — finds the gated launch flow (§1, row 1c) never
   called the shared helper at all; fixes it to call `runProviderLogin` too.
3. **`PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md`** — finds a fourth still-independent pair
   (`useGlobalLogin`/`loginViaTerminal`) and the New Agent modal's structurally separate flow (§1, row 1d).
   Phases the remaining work; Phase 3 (merging the New Agent modal flow into `runProviderLogin`) is
   explicitly deferred pending a design spike.

**One correction to that plan doc, verified directly against the PR's actual committed code rather than
its own prose:** the plan doc's Phase 3 section describes a "decided interim stopgap" where leaving the
New Agent modal's Identity field blank is treated as a supported ambient-creds path. **That text is
stale.** The PR's actual `AgentLaunchModal.tsx` (verified via `git show` against the PR branch) requires
`accountId() !== ""` in both `authBlocksLaunch()` and `canSubmit()` for every oauth-class provider — this
was deliberately reverted later the same session the plan doc was written, once the policy shifted to
"every oauth-class agent requires a real bound account, enforced everywhere, no exceptions," and the plan
doc's paragraph was never updated to match. Anyone reading that doc standalone would draw the wrong
conclusion about what the PR actually does today.

**Net effect if merged as-is:** four of five duplicated implementations collapse into one orchestrator.
The New Agent modal flow (row 1d) remains separate, and — more importantly for this report — the
underlying mechanism for every tier-3 fallback is still "spawn a visible OS terminal window," now just
reached from fewer call sites. That's a real, worthwhile fix for the fragmentation problem. It is not an
architecture rethink; it consolidates the existing pattern rather than replacing it.

---

## 4. The other two OAuth systems (for completeness — not touched by PR #2255)

### 4.1 MuxBus / AgentMux Cloud

Fully independent — zero shared code with the identity system (confirmed by direct grep of both module
trees). Cognito PKCE, one **global singleton row** (`db_muxbus_credentials`, `id='global'`) — not
per-agent, not per-provider. **Notably, its tokens are stored as plaintext TEXT columns directly in SQLite
— no `SecretRef`/keychain indirection at all**, unlike every credential in the `db_accounts` system. This
session already fixed one incident here (a long-lived WebSocket connection never proactively re-checking
wall-clock expiry, only trusting the token was fine because the socket stayed connected) by wiring a
proactive refresh into the existing ping-interval tick — a small, local instance of exactly the "proactive
refresh, single scheduler" pattern §5.4/§6 recommend generalizing.

### 4.2 Armory service-account OAuth (`identity/oauth_client.rs`)

A third, separate OAuth implementation for "connect a Google/Microsoft/GitHub/Slack account" buttons —
PKCE loopback for Google/Microsoft, RFC 8628 device flow for GitHub, PKCE+BYO-secret for Slack. All
`client_id`s are currently `None` in the static catalog — this is a **scaffold pending provisioned
credentials**, not a live system, driven by yet another separate RPC namespace
(`account.oauth.start/poll/cancel`) distinct from both the CLI-provider `auth.*` namespace and MuxBus.
Worth noting: this scaffold **already implements RFC 8628 device flow for one provider (GitHub)** — the
exact pattern §5/§6 recommend generalizing to the CLI-provider system already has a partial implementation
sitting unused elsewhere in the same codebase.

---

## 5. External research — how comparable tools solve this

Full research with inline citations: see the companion research pass (summarized here; ask if you want the
complete source document promoted into the repo too). Five threads, condensed:

### 5.1 The headless-OAuth problem: nobody solves it inside the headless process — they route around it

Every comparable orchestrator (OpenClaw, Docker-based Claude Code wrappers, `opcode`) either reuses an
already-completed host-level login or requires interactive setup once outside the managed/headless
context — none trigger fresh OAuth from inside a spawned, console-less process. CI/CD integrations
sidestep the problem entirely by using long-lived tokens or raw API keys instead of consumer-subscription
OAuth. Anthropic's own official prior art for "no local browser" (`claude remote-control`) makes only
outbound HTTPS calls and never opens a local port — but requires an *already-authenticated* session as a
precondition, it doesn't solve first-login.

**Per-CLI OAuth mechanics, as of July 2026:**
- **Claude Code**: PKCE + local HTTP callback listener. No RFC 8628 device flow exists. Two feature
  requests for one are open and unresolved (#22992, #20215); #22992 documents users' actual current
  workaround as installing a full desktop environment on a headless box just to click through OAuth once —
  i.e., AgentMux's visible-terminal hack is not an outlier, it's the same genre of workaround the whole
  ecosystem keeps landing on absent device flow. Working non-interactive bypasses: `claude setup-token`
  (long-lived OAuth token) and `ANTHROPIC_API_KEY` for `-p`/headless mode.
- **Codex CLI**: PKCE + local callback on a **fixed** port (1455). Device flow (`--device-auth`) genuinely
  exists but is **opt-in-gated per account or by a workspace admin** — without that opt-in, headless/SSH
  contexts hit a hard block with no fallback (issue #9253).
- **Gemini CLI**: browser OAuth, no stable device flow; a URL-fallback for headless contexts has regressed
  across point releases at least once (P0 issue #13853) — evidence that even where a workaround exists,
  it isn't durable across upstream CLI updates AgentMux doesn't control.

### 5.2 RFC 8628 (Device Authorization Grant) is the structurally correct fix

Purpose-built for exactly this situation: a device prints a code + URL, the user completes auth on **any**
separate browser/device, the device polls for a token. No console requirement, no localhost listener, no
platform-specific "spawn a terminal" logic — identical code on Windows/macOS/Linux. Already the default or
standard flow in `gh auth login`, `docker login` (default unless `--username` passed), `az login
--use-device-code`, `aws sso login --use-device-code`, `kubectl oidc-login --grant-type=device-code`.
**Caveat, stated plainly:** none of AgentMux's three target CLIs expose this unconditionally today — this
argues for AgentMux building its own device-flow-shaped shim (following community precedent like
`opencode-openai-device-auth`), talking to each provider's OAuth server directly where possible, rather
than waiting on upstream CLI support that may never arrive uniformly.

### 5.3 Multi-account credential management — avoid stateful "active account" switching

Cross-tool lesson from `gh`, `gcloud`, AWS CLI, rclone: a shared, mutable "current active account" pointer
(`gh auth switch`, `gcloud config set account`) is a race condition waiting to happen the moment two
sessions run concurrently — exactly AgentMux's shape (many panes, potentially many accounts, running at
once). The tools that avoid this bug class use **per-invocation, explicit binding** instead — an env var
override (`GH_TOKEN`, `AWS_PROFILE`) or address-embedded selection (rclone's `remote:path`) rather than a
global pointer any process could accidentally read after another process changed it.

**1Password's Claude Code shell plugin is the closest real prior art found** — session-scoped credential
injection with an explicit precedence order (terminal-session → directory → global), never touching the
target tool's own config format, vanishing when the subprocess exits. Worth studying directly as a
reference design, not just an analogy, since it's solving nearly this exact problem today.

### 5.4 OS-native secret storage

Windows Credential Manager, macOS Keychain, and Linux Secret Service are all viable, all wrappable via the
Rust `keyring` crate — but Linux Secret Service assumes a running desktop session + D-Bus + keyring daemon,
**routinely absent on headless Linux**, which should be treated as an expected condition with a real
fallback, not an edge case. The harder, AgentMux-specific problem is the **handoff**: the target CLI
expects to read a plaintext credentials file from a config-dir path, so the secret can't stay purely inside
the OS keychain — it has to be materialized to disk momentarily for the child process. Best practice
(validated by Kubernetes Secret volumes, Git Credential Manager's plaintext-fallback mode): materialize
into a directory that's verified tmpfs where available (Linux), `0600`/current-user-ACL everywhere, with
DPAPI-style encryption-at-rest as defense in depth where no memory filesystem exists (Windows/macOS), a
`Drop`-guard cleanup path for the common case, **plus a startup sweep for stale files** — since crash/
`SIGKILL` can never be fully covered by destructor-based cleanup alone.

### 5.5 Token refresh — proactive, single-flight, and why this matters concretely (not hypothetically)

Industry convergence: layer **proactive** refresh (a background timer per credential, refreshing at
~5-minutes-absolute or 70-80%-of-lifetime, whichever fits the provider's TTL) as the primary mechanism,
with reactive refresh-on-401 kept only as a safety net. The dangerous failure mode is **concurrent
duplicate refresh** — with providers that rotate refresh tokens on each use (which Anthropic/OpenAI/Google
all effectively do in some form), two simultaneous refresh attempts for the same credential can trigger
reuse-detection and revoke the *entire token family*, forcing full re-auth. This is not a theoretical risk
for AgentMux specifically: **Claude Code itself has two open, unresolved production issues from exactly
this bug class** — #25609 (multiple concurrent CLI sessions sharing one credentials file with no locking,
~3x/week auth loss reported) and #29896 (a failed refresh overwriting a valid credential with an empty
value, causing 24/7 agents to lose auth roughly every 12 hours). Any AgentMux design that runs multiple
sessions against the same bound account needs a **single-flight guard keyed by credential identity**
(account+provider) — the same fix Google's own `google-auth-library` and Go's `singleflight` package both
implement — or it inherits this exact failure mode on top of Claude Code's own.

### 5.6 The "credential broker" pattern, named and precedented

Converges on the term **"credential broker"** (HashiCorp Vault's own framing, CyberArk's "Secretless
Broker," 1Password's newer "Credential Broker" product) — a centrally-addressable service consumers
explicitly call into, distinct from AWS's **"credential provider chain"** pattern (an ordered, *ambient*
fallback search evaluated client-side) — worth flagging explicitly because a provider chain is
architecturally closer to the "ambient fallback" anti-pattern AgentMux is already removing than to the fix
for it. Canonical responsibilities converge across Vault, Kubernetes' TokenRequest API, SPIFFE/SPIRE,
Netflix's BLESS/Lemur, and 1Password's Credential Broker: **issue** (scoped to one caller) → **cache** →
**proactively refresh** → **revoke on demand / cascade-revoke on session teardown** → **hard-isolate per
consumer** → **audit**. SPIFFE/SPIRE's workload-attestation model (binding a credential to a specific
running process by PID/binary/session attributes, refusing to hand it to anything else) is the strongest
precedent for making "session X can only use credential Y" a structural invariant rather than a
conventionally-followed rule.

---

## 6. Recommended target architecture

### 6.1 One Credential Broker, not three systems

Consolidate provider-CLI identity, MuxBus, and the Armory service-account scaffold behind one backend
component with a single interface shape: `getCredential(sessionId, provider) -> materialized handle`,
pull-based (cache-check → refresh-if-stale → fetch-if-absent), modeled after the AWS SDK's
`RefreshableCredentials` pattern. This doesn't mean merging their data models overnight (MuxBus's
single-global-row shape is legitimately different from per-account CLI credentials) — it means one
component owns *issuance, storage-backend selection, refresh, and revocation* for all of them, so a fix
like this session's MuxBus proactive-refresh patch is implemented once, not rediscovered per-system.

### 6.2 Replace the terminal-spawn workaround with a device-flow-shaped login shim

Don't wait on Claude Code/Codex/Gemini to ship reliable device flow. Build AgentMux's own device-flow
shim per provider — talk to each provider's OAuth server's device-authorization endpoint directly where it
exists at the protocol level even if the CLI's own UX doesn't expose it, and fall back to each CLI's
documented non-interactive path otherwise (`claude setup-token`/`CLAUDE_CODE_OAUTH_TOKEN`, `codex login
--with-api-key`, Gemini's API-key/ADC path) as the primary managed-session mechanism, with a real
interactive browser login treated as a one-time onboarding step performed outside any headless spawn.
Notably, **AgentMux already has a working RFC 8628 device-flow implementation for GitHub** sitting mostly
unused in `identity/oauth_client.rs` — the shim doesn't need to be invented from nothing, it needs the
existing pattern generalized and pointed at the CLI-provider system.

This one change eliminates the entire "spawn a visible OS console, platform-specific code times three"
category, which today is duplicated (or missing) across all five login-triggering code paths in §1.

### 6.3 Storage: OS-native keychain as source of truth, momentary plaintext handoff only

Keep the long-lived secret only in the OS-native store behind a pluggable backend interface (the `keyring`
crate covers all three OSes; the Docker credential-helper `get`/`store`/`erase`/`list` protocol is a good
model for the interface shape so plaintext-fallback and future backends stay swappable). Materialize into
the CLI's expected config-dir file only at spawn time: verified-tmpfs on Linux where available, `0600`
everywhere, DPAPI-wrap-at-rest on Windows as defense in depth. Explicitly design "no Secret Service
available" as an expected Linux condition with a working (if weaker) fallback, not an error state — this
directly generalizes MuxBus's current plaintext-in-SQLite storage (a real gap relative to the
`SecretRef`-backed system next to it) into the same keychain-backed model everything else should use.

### 6.4 Refresh: one proactive, single-flight-guarded scheduler per credential

A single background actor per credential (not per-session, not per-call-site) that refreshes on a hybrid
margin (~5 min absolute buffer, 70-80%-of-lifetime for providers with unusual TTLs), single-flight-guards
concurrent refresh attempts keyed by (account, provider) so N sessions sharing one account collapse onto
one refresh call, and — critically, per §5.5 — **preserves the last-known-good credential on a failed
refresh rather than overwriting it**. This is the generalized form of the MuxBus fix already shipped this
session, applied everywhere, and it's the direct structural defense against the exact failure mode Claude
Code's own #25609/#29896 document.

### 6.5 Per-session isolation as a structural invariant, not a policy

The spawn gate in `resolver.rs` already does real enforcement work (§2.4) — the target architecture should
generalize its shape (bind a credential to a specific session/process at spawn time, refuse to hand it to
anything else) as the Credential Broker's own responsibility, borrowing SPIFFE/SPIRE's workload-attestation
framing: attest a spawned CLI subprocess by session ID/process handle at spawn time, and have the broker
structurally refuse cross-session credential reuse — converting "every oauth-class agent needs a real
bound account" from a spawn-time check one call site enforces into an invariant the broker itself can't be
bypassed on, no matter how many future login-trigger code paths get added.

### 6.6 What this replaces, concretely

| Today | Target |
|---|---|
| 5 independent login-trigger implementations | 1 orchestrator + 1 broker interface, all triggers call the same thing |
| 2 CEF-host login primitives + 1 separate `agentmux-srv` `auth.*` RPC surface | 1 broker-owned login flow, one RPC surface |
| Visible-terminal spawn as the OAuth fallback, platform-specific ×3 | Device-flow shim, identical across platforms |
| Plaintext MuxBus tokens in SQLite; `SecretRef`-backed CLI credentials; no storage layer for the service-account scaffold | One keychain-backed storage interface for all three |
| Reactive-only refresh (CLI-provider system), now-proactive-but-bespoke (MuxBus, fixed this session) | One proactive, single-flight-guarded scheduler generalized to every credential |
| `use_ambient_login` flag + spawn-gate check as the only isolation mechanism | Broker-level structural isolation, gate becomes one caller of it rather than the sole enforcement point |

---

## 7. Open questions / decisions needed

1. **Scope of v1**: does the Credential Broker rethink start as a rewrite of the CLI-provider system only
   (matching PR #2255's existing scope), or does it deliberately fold in MuxBus and the service-account
   scaffold from the start, given they're architecturally straightforward to unify but currently untouched
   by any in-flight work? Recommendation: design the broker interface to cover all three from day one even
   if MuxBus/service-account migration is phased later — retrofitting a unified interface onto a
   CLI-provider-only design later is exactly the kind of rework this report is trying to avoid repeating.
2. **Device-flow shim scope**: build against each provider's OAuth server directly (requires reverse-
   engineering or documented device-authorization endpoints per provider — confirmed to exist for GitHub
   already via the unused `oauth_client.rs` scaffold; unconfirmed for Anthropic/OpenAI/Google at the raw
   protocol level) versus relying on each CLI's own non-interactive bypass (`setup-token`, `--with-api-key`,
   API keys) as a nearer-term stopgap while the shim is built. Recommendation: ship the CLI-native
   non-interactive bypasses first (lower effort, no reverse-engineering risk), treat the full device-flow
   shim as the follow-up that removes the last dependency on any interactive terminal step.
3. **`gemini`/`copilot` gate scope**: should these two providers, currently oauth-typed in the UI but
   excluded from the Rust spawn gate and isolation-dir minting, be brought into full parity with
   `claude`/`codex`/`openclaw`, or is their exclusion intentional? This should be resolved before or during
   the broker build, not carried forward as unexamined drift.
4. **PR #2255's fate**: land it as-is (a real, valuable fragmentation fix, ships sooner) and treat this
   report as the next phase of work, or fold its remaining scope directly into the broker rewrite and skip
   an intermediate consolidation step? Recommendation: land #2255 — the fragmentation fix is independently
   valuable and de-risks the broker work by reducing five call sites to migrate down to one.

---

## 8. Phase D conclusion — device-flow shim is not viable for any target provider, confirmed by direct evidence

§6.2 recommended building AgentMux's own RFC 8628 device-flow shim rather than waiting on native CLI
support, scoped to the three providers that motivated it — "Claude Code/Codex/Gemini." Before building it,
a dedicated feasibility spike checked the one thing the original research flagged as unresolved for each of
those three: independent of the CLI's own command-line UX, does the provider's underlying OAuth
**authorization server** support the Device Authorization Grant at the protocol level — i.e. could AgentMux
call a `device_authorization_endpoint` directly, bypassing the CLI's UX entirely?

**Anthropic: no.** `claude.ai/.well-known/openid-configuration` returns only the SPA shell (no JSON);
`platform.claude.com/.well-known/openid-configuration` 404s; `console.anthropic.com` redirects to
`platform.claude.com`. The reverse-engineered `querymt/anthropic-auth` project and Claude Code's own
documented flow show PKCE + `authorization_code` + `refresh_token` only, against
`console.anthropic.com/oauth/authorize` (auth) and `platform.claude.com/v1/oauth/token` (token) — no
device grant anywhere. Claude Code's own client code already *probes* for a `device_authorization_endpoint`
in server metadata (the mechanism behind open feature requests anthropics/claude-code#22992/#20215), and
those requests stay open precisely because the server never advertises one. Anthropic's own Feb-2026 legal
update further restricts OAuth usage to Claude Code/Claude.ai and actively blocks third-party clients
(cited case: OpenCode). **A device-flow shim against Anthropic's server is not just hard, it's currently
impossible** — there is nothing to call.

**OpenAI: a real endpoint exists, but the gate is server-side, so a shim gains nothing.** A working device
endpoint exists at `https://auth.openai.com/codex/device` (Codex-specific, not advertised in the standard
OIDC discovery doc, which lists `grant_types_supported: ["authorization_code","refresh_token"]` only). The
community project `tumf/opencode-openai-device-auth` already drives it from outside the Codex CLI, reusing
Codex's own public client id — confirming AgentMux wouldn't need its own registered client. **But** the
opt-in gate (Settings → Security → "Allow device code login", or workspace-admin-enabled) is enforced by
the **server**, not the CLI: Codex issue #9418 shows the *server itself* returns "Please contact your
workspace admin to enable device code authentication" for un-opted-in accounts. A custom AgentMux shim
calling the same endpoint would hit the identical gate for the identical users the existing gated
`codex login --device-auth` flag already fails for — zero net benefit over what's already there.

**Gemini: the protocol-level endpoint is real, but the existing client can't use it — a shim means
registering and maintaining a new Google OAuth app, not reusing what's there.** Google publishes a fully
documented, general-purpose device authorization endpoint at `https://oauth2.googleapis.com/device/code`
(token exchange at `https://oauth2.googleapis.com/token`) as part of its standard "TV and Limited-Input
Device" OAuth flow — the strongest starting position of the three providers, since this is official, stable
Google infrastructure rather than a reverse-engineered or CLI-specific mechanism. The blocker is the OAuth
**client**, not the server: Google ties device-grant eligibility to how a client is registered (it must be
created as a "TV and Limited Input" client type, distinct from the "Desktop app" type), and Gemini CLI's own
public client id (`681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com`) is not
registered that way — corroborated by an open, unresolved feature request in Gemini CLI's own repo asking
for exactly this headless/device-code capability, which would already be closed if their existing client
supported it. That means AgentMux cannot reuse Gemini CLI's client id the way `tumf/opencode-openai-device-auth`
reused Codex's; a Gemini shim would require registering and operating AgentMux's own Google Cloud OAuth
client configured for limited-input devices, which pulls in Google's app-verification/consent-screen review
for the scopes Gemini CLI needs. That is a standalone, ongoing product commitment (a Google-verified
AgentMux-branded OAuth app), not a lightweight shim over infrastructure that already exists for AgentMux's
use — a materially different (and larger) lift than the two-line client-id reuse that works for OpenAI.

### Decision: do not build the device-flow shim

Confirmed with concrete, cited evidence for all three originally-scoped providers — not a hedge. §6.2's
recommendation is superseded by this finding: Anthropic has no endpoint to call, OpenAI's endpoint is
gated server-side with zero net benefit over the CLI's own gated flag, and Gemini's endpoint would require
standing up and maintaining a new, independently-verified Google OAuth client rather than reusing existing
infrastructure — none of the three clears the bar for "build our own shim" today.

**On the plan's own fallback clause** ("wire each CLI's own documented non-interactive bypass —
`claude setup-token`, `codex login --with-api-key` — instead"): on closer inspection this is not the small
follow-up task it first looked like, either:

- `claude setup-token` still requires a real browser-capable environment to complete — it doesn't remove
  the headless problem, it just produces a different artifact (a portable long-lived token string, printed
  to stdout, instead of a token file written to disk). AgentMux's existing `open_login_terminal` fallback
  (a real, visible, un-piped console) already provides that browser-capable environment for regular
  `login`/`auth login` today; running `setup-token` there instead would work identically for completing the
  OAuth handshake, but the resulting **printed token** can't be captured the way `run_cli_login`'s piped
  stdout scrape captures a URL — `open_login_terminal` is deliberately un-piped (piped is exactly what
  breaks the browser-open in the first place). Consuming a `setup-token` output would need a genuinely new
  UI mechanism (e.g. generalizing the existing URL-paste box into a token-paste box) — a legitimate,
  separately-scoped enhancement, not a natural extension of Phases A–C.
- `codex login --with-api-key` isn't a drop-in fallback either: it forfeits ChatGPT-subscription-tier
  access in favor of pay-per-token API billing — a real tradeoff the user should choose explicitly, not
  something AgentMux should silently substitute when OAuth is inconvenient.

**Recommendation:** treat Phase D as concluded, not deferred. The terminal-window fallback that Phases A–C
already build on (and that predates this rethink) remains the correct primary mechanism for headless-login
recovery for all three providers going forward. A "paste a pre-generated long-lived token" feature (for
Claude/Codex) and a "register AgentMux's own verified Google OAuth client" project (for Gemini) are both
legitimate ideas for future, independently-scoped work if either keeps coming up in practice — neither is
part of this rethink's core deliverable, and building either now, rushed, without its own design pass,
would be worse than not building it.

---

## Appendix: source material

- Full file:line-cited codebase map (auth-codebase-map.md, ~700 lines) and full external best-practices
  research with inline citations (auth-best-practices-research.md, ~240 lines) were compiled as working
  documents for this report. Ask if you want either promoted into the repo verbatim alongside this
  synthesis — they contain considerably more granular detail (e.g. exact struct definitions, every
  migration involved, the complete provider-by-provider CLI auth mechanics writeup with ~40 source URLs)
  than is reproduced here.
