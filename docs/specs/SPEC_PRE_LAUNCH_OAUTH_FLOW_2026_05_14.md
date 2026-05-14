# Spec: Pre-launch OAuth flow — identity-first agent setup

**Date:** 2026-05-14
**Author:** AgentA
**Status:** Draft
**Companion to:** [`SPEC_OAUTH_IN_IDENTITY_BUNDLES_2026_05_13.md`](SPEC_OAUTH_IN_IDENTITY_BUNDLES_2026_05_13.md) — that spec covers *where* OAuth tokens are stored and how they refresh. This spec covers *when in the user flow* OAuth happens and how it binds to the agent.

---

## 1. Driving requirement

> When I first launch an agent in AgentMux, the "Launch" button should be greyed out until I connect an identity for that provider via OAuth. A "Connect with OAuth" call-to-action sits in front of Launch. Authorizing creates an Identity bundle automatically; that bundle is bound to the agent I just launched. Future launches reuse the same bundle. New agents can optionally inherit existing bundles instead of forcing a re-auth.

In short: **auth gates launch**, not the other way around. Today the launch flow tries OAuth *during* launch (`launch-flow.ts` Phase 2). Move it ahead of the Launch button.

---

## 2. Today's state (problem statement)

The launch sequence in `frontend/app/view/agent/flows/launch-flow.ts`:

1. **Phase 0** — container runtime check (host agents skip)
2. **Phase 1** — CLI detection / npm install
3. **Phase 2** — auth check → if not authenticated, *spawn login command in-line*, poll for 5 min, hope the user finishes the OAuth dance in their browser
4. **Phase 3** — controller registration

Concrete issues this produces:

- **The "Launch" button is a lie.** A user clicks Launch with no auth set up; the modal goes through CLI install, then suddenly opens an OAuth browser tab. The user didn't agree to that — they thought they were clicking "start the agent."
- **Auth state is scoped to the agent's working dir.** Per provider, `CLAUDE_CONFIG_DIR=<workdir>/.claude` etc. — so a fresh agent gets a fresh auth surface. Logging in once doesn't help the next agent.
- **No first-class identity model in the launch UI.** Identity bundles exist in the schema (v7, [#746](https://github.com/agentmuxai/agentmux/pull/746)) and have their own pane, but the launch modal doesn't *require* one — users default to "blank singleton" which means "use ambient creds, hope for the best."
- **URL fallback is implicit.** `launch-flow.ts` already captures an OAuth URL from the CLI's stdout and surfaces it via `setAuthUrl` so the user can copy/paste — but only if the CLI happens to print it. No universal fallback exists.
- **Auth timeout collides with normal user pace.** 5-minute polling means a user who steps away mid-OAuth gets a "launch failed" message and has to retry from scratch.

What's *right* today (and we keep):

- Per-provider auth isolation env vars (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GEMINI_CLI_HOME`, etc.) are already plumbed through.
- Identity bundle schema exists (`db_identities` + `db_identity_accounts` + `db_identity_bindings`).
- The companion storage spec defines where OAuth tokens persist.

---

## 3. Proposed UX flow

### 3.1 First-time launch (no bundle exists for this provider)

```
┌─────────────────────────────────────────────────────────────────────┐
│  Launch Agent                                                       │
│                                                                     │
│  Provider:  [Claude Code     ▼]      Identity: [Blank singleton ▼] │
│                                                                     │
│  ⚠ Claude Code requires an OAuth login before launch.              │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │             🔐  Connect with OAuth                          │   │
│  │                                                             │   │
│  │   Opens browser → Anthropic login → returns to AgentMux.   │   │
│  │   Tokens get saved into a new Identity bundle so the next  │   │
│  │   agent doesn't have to re-authenticate.                   │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  [Browser didn't open? Use auth URL]                                │
│                                                                     │
│                                          [ Launch ]  [Cancel]      │
│                                            ^^^^^^^                  │
│                                            disabled                 │
└─────────────────────────────────────────────────────────────────────┘
```

User clicks **Connect with OAuth**. The browser opens; if it doesn't, the **Use auth URL** link expands an inline panel with the URL + Copy button.

### 3.2 Mid-OAuth state

```
┌─────────────────────────────────────────────────────────────────────┐
│  Launch Agent                                                       │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │   🔐  Waiting for OAuth …                                   │   │
│  │                                                             │   │
│  │   1. Authorize AgentMux in your browser tab.                │   │
│  │   2. We'll detect the redirect and continue.                │   │
│  │                                                             │   │
│  │   URL not opening? Copy this and paste anywhere:            │   │
│  │   https://console.anthropic.com/oauth/authorize?...  [📋]  │   │
│  │                                                             │   │
│  │                                          [ Cancel login ]  │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│                                          [ Launch ]  [Cancel]      │
│                                            disabled                 │
└─────────────────────────────────────────────────────────────────────┘
```

### 3.3 Post-auth (bundle exists, ready to launch)

```
┌─────────────────────────────────────────────────────────────────────┐
│  Launch Agent                                                       │
│                                                                     │
│  Provider:  [Claude Code     ▼]                                    │
│  Identity:  [Claude (asaf@example.com) ▼]   ✓ Authenticated        │
│                                                                     │
│  Instance name (optional): [___________________]                    │
│                                                                     │
│                                          [ Launch ]  [Cancel]      │
│                                            ^^^^^^^^^               │
│                                            enabled                  │
└─────────────────────────────────────────────────────────────────────┘
```

The bundle the user just authenticated against is auto-selected. Launch enables.

### 3.4 Second launch (bundle exists, just pick it)

```
Identity:  [Claude (asaf@example.com) ▼]
           │  Claude (asaf@example.com)  ← default for new agents
           │  Claude (work-account)
           │  ─────────────
           │  + Connect another OAuth account
           │  ⊗ Blank singleton (ambient creds)
```

Users can pick a different existing bundle, or **+ Connect another OAuth account** to walk through 3.1 → 3.2 → 3.3 again with a new bundle name.

### 3.5 Provider mismatch

If a user picks a bundle that doesn't have an account for the selected provider, surface inline:

```
Provider:  [Codex CLI         ▼]
Identity:  [Claude (asaf@example.com) ▼]
           ⚠ This bundle has no Codex account. [ Connect Codex to this bundle ]
                                                ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
                                                inline action
```

Clicking re-runs OAuth scoped to **add a new account row to the existing bundle**, not to create a new bundle.

---

## 4. Per-provider auth landscape

`frontend/app/view/agent/providers/index.ts` defines 7 providers. Auth flow varies meaningfully:

| Provider | `authType` | Real-world flow | Browser-open? | URL fallback? |
|---|---|---|---|---|
| Claude Code | `oauth` | Anthropic OAuth2 + PKCE → `localhost` callback (~Claude CLI's local listener) → writes `~/.claude/credentials.json` | Yes, CLI auto-opens | URL printed in stdout |
| Codex CLI | `oauth` | OpenAI OAuth2 → callback | Yes | Less reliable |
| Gemini CLI | `oauth` | Google OAuth2 + Cloud project context | Yes | Yes |
| GitHub Copilot | `oauth` | GitHub OAuth (CLI uses device flow — pastes a code) | No (device-flow) | Native — device code is the fallback |
| OpenClaw | `api-key` | User pastes key into onboarding wizard | N/A | N/A |
| Kimi Code | `api-key` | Same | N/A | N/A |
| Pi | `api-key` | Same | N/A | N/A |

Implications for the spec:

- **Three patterns to support**: browser-OAuth-with-callback (Claude/Codex/Gemini), device-code-flow (Copilot), API-key-paste (OpenClaw/Kimi/Pi).
- **The "Connect with OAuth" button label/copy must adapt**: for API-key providers it's "Paste API key"; for device-flow it's "Get pairing code."
- **All paths converge on the same outcome**: a new `IdentityAccount` row inside an `IdentityBundle` (existing or newly-created), with a `SecretRef` appropriate to the kind.

### Provider-specific browser-failure recovery

Best practices from OAuth ecosystem (RFC 8628 device authorization, [OAuth for Native Apps](https://datatracker.ietf.org/doc/html/rfc8252)):

- **Claude Code**: CLI already supports `--use-existing-callback` flag (per its docs) which prints the URL and waits on stdin for the callback URL pasted back. Mirror this UX in our wrapper: detect "callback failed to open browser" → show URL → user pastes redirect URL into our input → we send it back to the CLI's stdin.
- **Codex / Gemini**: Less documented, but both CLIs print the auth URL to stdout. Capture it via the existing `setAuthUrl` mechanism in `launch-flow.ts`. Add a textbox for the user to paste back the redirect URL.
- **GitHub Copilot**: Device code is the *only* flow — show the code prominently with a copy button and the verification URL. No browser dependency at all.
- **API key providers**: Modal with a password-style input. Validate by running the provider's `authCheckCommand` against the just-pasted key.

---

## 5. Identity bundle lifecycle

### 5.1 Bundle states

```
                     ┌──────────────────┐
                     │  Empty / new     │ ← user opens launch modal,
                     │  (default)       │   no existing bundle for this provider
                     └────────┬─────────┘
                              │  Click "Connect with OAuth"
                              ▼
                     ┌──────────────────┐
                     │  Awaiting auth   │ ← OAuth tab open, polling for redirect
                     └────────┬─────────┘
                  ┌───────────┴────────────┐
            success │                      │ timeout / cancel
                    ▼                      ▼
       ┌──────────────────┐     ┌──────────────────┐
       │  Authenticated   │     │  Empty / new     │
       │  (bundle saved)  │     │  (back to start) │
       └────────┬─────────┘     └──────────────────┘
                │
                │ User clicks Launch
                ▼
       ┌──────────────────┐
       │  Bound to agent  │ ← `db_agent_instances.identity_id = <bundle.id>`
       └──────────────────┘
```

### 5.2 New-bundle auto-creation

When a first OAuth completes:

1. Backend creates a `db_identities` row with `name = "<provider-display-name> (<email-from-token>)"`. Example: `"Claude (asaf@example.com)"`. User can rename later.
2. Backend creates a `db_identity_accounts` row linked to the new bundle via `db_identity_bindings`, with `kind = "oauth"` and the appropriate `SecretRef` variant (see companion storage spec §4).
3. Frontend reads back the new bundle id and selects it in the dropdown.

### 5.3 Adding to existing bundle

When user picks an existing bundle but clicks **+ Connect <provider>** for a provider missing from the bundle:

1. Same OAuth flow.
2. On completion, backend inserts a new `db_identity_accounts` row + binding under the *existing* bundle id.
3. Bundle name stays unchanged.

### 5.4 Reuse on subsequent launches

When the user selects a previously-authenticated bundle and clicks Launch:

1. Spawn-time resolver (`agentmux-srv/src/identity/resolver.rs`) reads `instance.identity_id`, looks up bindings for the requested provider, materializes credentials per the storage spec (§5).
2. Refresh tokens are valid → CLI starts successfully without prompting. Storage spec's watcher (§6) keeps captures fresh.
3. **No OAuth dialog appears.** This is the user-visible "I logged in once" promise.

### 5.5 Bundle expiration / re-login

If the storage spec's `last_refreshed_at_ms` exceeds the provider's known refresh-token TTL (e.g., Anthropic's documented ~90 days):

1. The Identity dropdown shows the bundle with a warning badge: `⚠ Re-login required`.
2. Selecting it disables Launch and surfaces a "Re-authenticate" CTA — same flow as 3.1 but pre-fills the bundle so we update its rows rather than create a fresh bundle.

---

## 6. Browser + URL fallback architecture

### 6.1 Happy path (browser opens)

```
┌────────────┐    1. spawn OAuth helper
│  Frontend  ├───────────────────────────────────────┐
└─────┬──────┘                                       ▼
      │                                       ┌─────────────┐
      │ 2. poll for auth                      │ Backend RPC │
      │    completion                         │ AuthStartOAuth
      │                                       └──────┬──────┘
      │                                              │ 3. spawn CLI
      │                                              ▼
      │                                       ┌─────────────┐
      │                                       │ Provider CLI│ ← opens browser
      │                                       │ (claude/...)│
      │                                       └──────┬──────┘
      │                                              │ 4. CLI's local
      │                                              │    callback receives
      │                                              ▼    redirect
      │                                       ┌─────────────┐
      │                                       │ Credentials │
      │                                       │ on disk     │
      │                                       └──────┬──────┘
      │  5. poll detects auth                        │
      │  ◄─────────────────────────────────────────  │
      ▼                                              ▼
┌────────────┐  6. capture creds → bundle  ┌──────────────┐
│  Frontend  │◄───────────────────────────── Storage spec │
└────────────┘    enable Launch              §6 watcher   │
                                            └──────────────┘
```

### 6.2 Browser-didn't-open path

```
┌────────────┐    1. spawn OAuth helper
│  Frontend  ├───────────────────────────────────────┐
└─────┬──────┘                                       ▼
      │                                       ┌─────────────┐
      │ 2. user clicks                        │ Backend RPC │
      │    "Browser didn't open"              │             │
      │                                       └──────┬──────┘
      │                                              │
      │ 3. capture URL from CLI stdout               │
      │ ◄─────────────────────────────────────────── │
      ▼                                              │
┌────────────┐                                       │
│  Frontend  │  4. user copies, pastes into          │
│  URL panel │     any browser (could be phone)      │
└─────┬──────┘                                       │
      │  5. user pastes back the resulting URL      │
      ▼     (the redirect with the code)            │
┌────────────┐                                       │
│  Frontend  │  6. POST redirect URL                ▼
└─────┬──────┘ ────────────────────────────► ┌─────────────┐
      │                                       │ Backend RPC │
      │                                       │ AuthCompleteOAuth
      │                                       └──────┬──────┘
      │                                              │ 7. write to CLI's
      │                                              │    stdin
      │                                              ▼
      │                                       ┌─────────────┐
      │                                       │ Provider CLI│
      │                                       │ completes   │
      │                                       │ token       │
      │                                       │ exchange    │
      │                                       └──────┬──────┘
      │  8. poll → success                           │
      ▼                                              ▼
   enable Launch                              Storage spec §6
```

Key: the CLI's OAuth implementation is left intact. We're not implementing OAuth2 ourselves — we orchestrate the CLI through `stdin` so users who can't trigger a browser-auto-open path still complete the flow inside AgentMux.

### 6.3 Device-flow path (Copilot)

```
┌────────────┐    1. spawn copilot auth login
│  Frontend  ├──────────────────────► CLI
└─────┬──────┘                          │
      │                                 │ 2. CLI emits:
      │                                 │    "Enter code XXXX-YYYY at"
      │                                 │    "https://github.com/login/device"
      │  3. parse + render               │
      │ ◄────────────────────────────── │
      ▼                                  ▼
┌────────────┐
│  Frontend  │  4. user follows URL on
│  Code +    │     ANY device, enters code
│  URL panel │
└─────┬──────┘
      │  5. backend keeps polling
      │     CLI's stdout until success
      ▼
   enable Launch
```

### 6.4 API key path (OpenClaw / Kimi / Pi)

```
┌────────────┐
│  Frontend  │  1. show password-input modal
│  Modal     │
└─────┬──────┘
      │  2. user pastes key
      ▼
┌────────────┐
│  Backend   │  3. run authCheckCommand against the key
│  validate  │     to confirm it works
└─────┬──────┘
      │  4. success → save as SecretRef::PlaintextDev
      │     (PR 2 of storage spec: encrypt with keyring)
      ▼
   enable Launch
```

---

## 7. Backend RPCs (additions)

New commands in `agentmux-srv/src/server/identity_handlers.rs`:

```rust
// Start OAuth — spawns the provider's `auth login` command, returns a
// session token the frontend uses to track this attempt. Captures the
// CLI's stdout for URL detection.
StartProviderAuth { provider_id: String, into_bundle_id: Option<String> }
  -> { session_id: String, auth_url: Option<String> }

// Poll an in-flight auth session. Returns one of:
//   - "pending"       — still waiting on user
//   - "url-available" — captured an OAuth URL the user can paste
//   - "code-emitted"  — device flow code (Copilot)
//   - "success"       — credentials captured; bundle_id ready
//   - "failed"        — CLI exited non-zero, timeout, or user cancelled
PollProviderAuth { session_id: String }
  -> { status: "pending" | "url-available" | "code-emitted" | "success" | "failed",
       auth_url?: String, device_code?: { code: String, verification_url: String },
       bundle_id?: String, error?: String }

// Submit a callback URL the user pasted back (browser-didn't-open path).
SubmitAuthCallback { session_id: String, callback_url: String }
  -> { accepted: boolean, error?: String }

// Cancel an in-flight auth session. Kills the spawned CLI.
CancelProviderAuth { session_id: String }
  -> {}

// Submit an API key for kind=api-key providers.
SubmitProviderApiKey { provider_id: String, into_bundle_id: Option<String>,
                       api_key: String, account_name: String }
  -> { success: boolean, bundle_id?: String, error?: String }
```

The 5-minute polling timeout in today's `launch-flow.ts` Phase 2 moves up into `PollProviderAuth` and shifts ownership to the modal — modal disables Connect button after timeout and surfaces a clear retry CTA, rather than reporting "launch failed."

---

## 8. Frontend state machine

`launcher.tsx` gains a new pre-launch substate:

```typescript
type AuthState =
    | { kind: "unauthenticated"; needsAuth: true }     // show Connect CTA
    | { kind: "waiting"; sessionId: string; urlVisible: boolean; deviceCode?: string }
    | { kind: "ready"; bundleId: string }              // enable Launch
    | { kind: "expired"; bundleId: string };           // show Re-authenticate CTA

const launchEnabled = () =>
    authState().kind === "ready" || authState().kind === "expired" && hasFallbackCreds;
```

Selecting a bundle from the dropdown:
- Bundle has account for selected provider AND `last_refreshed_at_ms` is fresh → `ready`
- Bundle has account but `last_refreshed_at_ms` is stale → `expired`
- Bundle has no account for this provider → `unauthenticated` with "Connect <provider> to this bundle"
- Blank singleton → `unauthenticated` with "Connect with OAuth (creates new bundle)"

---

## 9. Migration from today's auth-on-launch

Current code path (delete after this lands):

- `frontend/app/view/agent/flows/launch-flow.ts` Phase 2 — auth-during-launch loop. **Becomes a no-op** because pre-launch enforces that auth has already happened. Keep it as a defensive fallback for one release (logs a warning + does the existing behavior) before removing in a follow-up.

What stays:

- Per-provider auth isolation env vars (`CLAUDE_CONFIG_DIR`, etc.) — pre-launch flow writes credentials into the same per-bundle path (storage spec §4), so the existing env-injection in `identity/resolver.rs` works unchanged.
- `authCheckCommand` per provider — used at *poll* time to confirm the CLI sees the credentials we just captured.

### Compat for existing users

A user who upgrades and already had agents running with the old flow:

1. Existing `db_agent_instances` rows have `identity_id = "blank"` → ambient creds (their working dir's `.claude/credentials.json`).
2. The new launch modal warns: `⚠ Agent uses ambient credentials — connect to a bundle for portability?` with an inline migrate action.
3. The migrate action runs OAuth once, captures into a new bundle, updates `identity_id` on the row.
4. User who never migrates: ambient creds keep working until the working dir is deleted.

---

## 10. PR sequence

### PR A — Backend RPCs + session manager

- `agentmux-srv/src/identity/auth_session.rs` — in-memory session map, CLI spawn + stdout capture
- `agentmux-srv/src/server/identity_handlers.rs` — the 5 RPCs in §7
- Unit tests for: session timeout, URL extraction, callback injection, device-code parsing
- Per-provider stdout-pattern matchers for URL detection (provider table in `auth_patterns.rs`)

### PR B — Launch modal pre-auth UX

- `frontend/app/view/launcher/launcher.tsx` — new `AuthState` machine, Connect CTA, URL panel, device-code panel, API-key modal
- Identity dropdown enriched with bundle status (✓ / ⚠ / not connected)
- Launch button gated on `authState().kind === "ready"`
- Vitest unit tests for the state transitions

### PR C — Bundle auto-creation + binding

- Backend: on `success` event from PR A, insert `db_identities` + `db_identity_accounts` + `db_identity_bindings` rows in a transaction
- Email extraction from provider tokens (provider-specific — Claude has it in JWT claim, Codex in profile endpoint)
- Frontend: on RPC success, refresh bundle list + auto-select new bundle
- Integration test: end-to-end "no bundles → Connect → bundle exists → Launch enabled"

### PR D — Migration helper for legacy ambient-cred agents

- One-time migration prompt (modal on first launch with `identity_id == "blank"`)
- "Connect now" runs the same flow as PR B, but the success handler also updates the existing `db_agent_instances` row
- Defer the deletion of `launch-flow.ts` Phase 2 until PR D ships

### PR E — Device-flow polish (GitHub Copilot)

- Custom panel for device-code rendering with big code + Open Verification URL button
- Polish for "code expired" state (Copilot re-emits if user takes >15 min)

### PR F — Expired bundle handling

- `last_refreshed_at_ms` staleness check
- "Re-authenticate this bundle" action — runs OAuth into the existing bundle (no new bundle row)
- Per-provider TTL table (Anthropic ~90d, OpenAI ~30d, Google ~30d, GitHub effectively forever)

PRs A–C ship the user-visible feature. D–F harden + clean up.

---

## 11. Risks + mitigations

| Risk | Mitigation |
|---|---|
| CLI silently changes its OAuth URL format → URL extractor breaks | Per-provider regex with a fallback to "any `https://` line during the first 30s of stdout" |
| User pastes wrong URL into callback box → CLI errors → cryptic state | Validate URL before submit: parse + check for `code=` or `state=` param. Reject inline. |
| OAuth session leaks (user closes modal mid-flow) → orphan CLI proc | `CancelProviderAuth` on modal close. `tokio::spawn` task tied to session id; killed on session removal. |
| User has two bundles for the same provider; picks the wrong one → wrong email/account | Show email in the dropdown label. Confirm modal when the bundle's email doesn't match the user's current GitHub identity (if known). |
| API key validation makes a real API call against the key → first OAuth-like network round-trip | Use the provider's lightest "whoami" endpoint via `authCheckCommand`. Document this in the API key paste modal. |
| Provider's `auth login` blocks on user input even after credentials exist | Skip the spawn entirely if `authCheckCommand` already says authenticated for the cred path we'd materialize. |
| Device-flow code expires before user enters it | Auto-restart the device-flow CLI when the spawned proc exits with the "code expired" status; show a refreshed code in the UI. |

---

## 12. Acceptance criteria

- [ ] Launch button is **disabled** when the selected provider has no authenticated bundle and the user has selected blank singleton.
- [ ] Clicking **Connect with OAuth** spawns the provider's `auth login`, captures the URL, attempts auto-open in the system browser.
- [ ] If auto-open fails or the user clicks "Use auth URL", the URL is displayed inline with a Copy button. Pasting the resulting redirect URL back into the modal completes the flow.
- [ ] On success, a new Identity bundle is created (`db_identities` + accounts + binding rows in one transaction), auto-selected in the dropdown.
- [ ] Launch enables. Clicking it spawns the agent with `identity_id = <new bundle id>`. Storage spec's resolver materializes the credentials.
- [ ] Closing the modal mid-OAuth cancels the spawned CLI (no orphan).
- [ ] Subsequent launches with the same bundle skip OAuth entirely.
- [ ] Selecting a bundle that doesn't have an account for the selected provider surfaces the inline "Connect <provider> to this bundle" CTA.
- [ ] API-key providers (OpenClaw / Kimi / Pi) show a password-style paste modal instead of an OAuth flow.
- [ ] Device-flow provider (Copilot) shows the device code prominently with the verification URL.
- [ ] Existing agents with `identity_id == "blank"` keep working; user is prompted (non-blocking) to migrate to a bundle.

---

## 13. Open questions

1. **Should "Connect with OAuth" auto-launch the agent on success**, skipping the post-auth "now click Launch" step? Likely yes — but some users may want to inspect the bundle before launching. Default: auto-launch with an unobtrusive "Connecting account…" → "Launching agent…" status. User can cancel before Launch fires.
2. **Where does the OAuth URL fallback panel live** — inside the launch modal as a slide-out, or as a separate dialog? Slide-out keeps context but compresses the layout on smaller windows.
3. **Bundle naming** — auto-generated from email is decent but cumbersome (`Claude (asafebgi+aliasX@example.com)`). Should we offer a rename prompt immediately after first OAuth, or let users rename later via the Identity pane?
4. **Should the launch modal show the bundle's other-provider accounts** ("This bundle also has GitHub authenticated")? Useful for users who think in cross-provider personas (work-asaf vs personal-asaf). Could clutter the dropdown.
5. **Device flow on Claude/Codex/Gemini** — none currently support device flow out of the box. Worth exploring as a pure user-side fallback (we host a tiny local landing page that captures the redirect and shows the user a "paste this back" page).

---

## 14. References

- Companion spec: [`SPEC_OAUTH_IN_IDENTITY_BUNDLES_2026_05_13.md`](SPEC_OAUTH_IN_IDENTITY_BUNDLES_2026_05_13.md) — token storage + refresh
- v7 schema: [PR #746](https://github.com/agentmuxai/agentmux/pull/746) — Identity bundles + Memory
- Identity pane: [PR #750](https://github.com/agentmuxai/agentmux/pull/750)
- Launch modal current state: `frontend/app/view/launcher/launcher.tsx`, `frontend/app/view/agent/flows/launch-flow.ts`
- Provider table: `frontend/app/view/agent/providers/index.ts`
- Spawn-time resolver: `agentmux-srv/src/identity/resolver.rs`
- RFC 8628 — OAuth 2.0 Device Authorization Grant
- RFC 8252 — OAuth 2.0 for Native Apps (best practices for CLI tools)
