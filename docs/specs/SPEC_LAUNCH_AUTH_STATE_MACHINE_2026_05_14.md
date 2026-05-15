# Pre-launch auth — complete user stories + state machine

**Date:** 2026-05-14
**Author:** AgentA
**Status:** Design — supersedes the §8 sketch in `SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md` with a full enumeration of paths and an extended reducer state machine.

---

## 1. Why this spec

The existing AuthState reducer (PR B-1, merged via #849) covers the happy path: blank singleton → Connect → Waiting → Success → Launch. Smoke testing surfaced gaps:

- **No "name the bundle" step.** OAuth completes and we synthesize a bundle id — the user never gets to name it, the bundle row never actually exists in the DB. Bundle persistence (PR C) needs a 2-phase commit: CLI auth done **→ user names it → save**.
- **No story for non-blank bundles missing the provider account.** Selecting a bundle that has GitHub bound but not Claude is the same situation as the blank singleton (auth required), but the UI today just trusts it.
- **No re-auth path.** When a bundle exists but its creds expired, the flow should refresh the **existing** rows rather than create a new bundle.
- **Cancel / dropdown-swap mid-flight is implicit.** What happens if the user changes the identity dropdown mid-OAuth? Closes the modal? Clicks Connect again? These weren't pinned down — and a Solid `createEffect` re-fire wiping an in-flight session is exactly the kind of bug that surfaced in the last smoke run.

This spec enumerates every path, every state, every transition, and pins them with invariants. The state machine extension is then implementable as a focused reducer change with new tests, not a re-architecture.

---

## 2. Decision: keep the formal reducer state machine

**Recommendation:** yes — extend `AuthState` (slice #11 of the frontend reducer roadmap once it ships) rather than handling these states ad-hoc inside the modal.

**Rationale:**
- We already have one (PR B-1 merged). The pattern is settled.
- The transition surface is 7 commands × 8 kinds = 56 cells. Inline this in JSX and you get the bug class we hit on PR #848 / #849 / #850 (codex flagged 8 P1s across the family).
- Audit ring (`recordDispatch`) gives free debugging — every transition is queryable in the diagnostics panel.
- Idempotency + post-close-command-dropped gates already enforced uniformly.
- Tests pin the cross-product. The reducer's `.test.ts` is the contract document.

**What stays out:** the **side effects** (RPC calls, browser open, DOM focus) live in the controller (`AuthFlowController`), not the reducer. The reducer is pure: command in → `{ state, events }` out. Today's `auth-flow-controller.ts` is the right home for the side-effect orchestration; this spec only extends what the reducer tracks.

---

## 3. User stories — complete enumeration

Stories are tagged `S<n>`. Each lists:
- Preconditions
- Steps
- Expected state machine transitions
- Final Launch button state (E = enabled / D = disabled)

### Happy path stories

**S1 — First-time launch, no bundles exist**
- Preconditions: only `blank` singleton in `db_identities`. User picks an agent card.
- Steps:
  1. Modal opens, dropdown = Blank, no other options.
  2. Connect-to-`<provider>` CTA visible. Launch = D.
  3. Click Connect. → `waiting`, browser opens.
  4. User authorizes in browser. CLI exits successfully.
  5. State → `authenticated { email }`. SaveBundle panel appears with default name `<ProviderDisplayName> (<email>)`.
  6. User accepts the default name, clicks **Save**.
  7. Backend commits 3 rows in a transaction. State → `ready { bundleId }`.
  8. Identity dropdown refetches; new bundle appears + auto-selects.
  9. Launch = E.

**S2 — Returning user, valid bundle selected**
- Preconditions: bundle "Claude (asaf@x.com)" exists with binding for `claude` provider, not stale.
- Steps:
  1. Modal opens, dropdown = "Claude (asaf@x.com)".
  2. No CTA. Launch = E from the start.
  3. User clicks Launch.

**S3 — Continuing a past agent**
- Preconditions: agent has prior instance with `identity_id="bundle-N"`.
- Steps:
  1. User picks the Continue dropdown. Modal pre-fills name + identity to the saved values.
  2. `isContinue = true` → entire auth gate is bypassed (we trust prior launch).
  3. Launch = E.

**S4 — API-key provider, no key configured**
- Preconditions: provider authType=`api-key` (openclaw / kimi / pi). Blank selected.
- Steps:
  1. Modal opens. ApiKey paste panel visible. Launch = D.
  2. User pastes key + account name, clicks **Save API key**.
  3. Backend `auth.submitapikey` validates via `authCheckCommand`, transitions to `authenticated`.
  4. SaveBundle panel appears with default name `<ProviderDisplayName> (<accountName>)`.
  5. User clicks Save → commit → `ready { bundleId }`. Launch = E.

### Unhappy-path stories

**S5 — User cancels during OAuth waiting**
- Preconditions: in `waiting`, browser open, CLI subprocess running.
- Steps:
  1. User clicks **Cancel login** in the modal.
  2. State → `unauthenticated`. Controller fires `auth.cancel` RPC — backend kills CLI subprocess.
  3. Browser tab may still be open; user closes it manually. (Out of scope to auto-close.)
  4. Connect CTA reappears.

**S6 — User closes the modal during OAuth waiting**
- Preconditions: in `waiting`, browser open, CLI subprocess running.
- Steps:
  1. User presses ESC / clicks backdrop / clicks Cancel on the launch modal.
  2. `PreLaunchAuthPanel` unmounts, `onCleanup` fires `controller.dispose()`.
  3. `dispose()` fires `auth.cancel` (already implemented in #850).
  4. Backend kills CLI. No orphan subprocess.

**S7 — User swaps identity dropdown during OAuth waiting**
- Preconditions: in `waiting` for bundle creation (selected = blank), CLI running.
- Steps:
  1. User changes dropdown to an existing bundle "Codex (asaf@x.com)" (different provider).
  2. State → `selected.providerId/bundleId` updated. The in-flight Claude session is now stale.
  3. Controller fires `auth.cancel` for the previous session, transitions to the new selection.
  4. State recomputes for the new selection: if the new bundle is valid for the new provider, `ready`; else `unauthenticated`.

**S8 — User clicks Connect a second time mid-flight**
- Preconditions: in `waiting`, another Connect click arrives (rapid double-click or bug).
- Steps:
  1. Reducer `ConnectClicked` while `state.kind !== unauthenticated/expired/failed` → **dropped** (`post-close-command-dropped`). No state change.
  2. No duplicate `auth.start` fires.

**S9 — OAuth times out (backend session 10-min cap)**
- Preconditions: in `waiting`, CLI never gets the redirect (user abandoned).
- Steps:
  1. Backend session manager fires `Failed { error: "timeout" }` after 10 min.
  2. Next frontend poll returns `failed`. State → `failed { error: "timeout" }`.
  3. FailedBanner with Retry button. Click Retry → new Connect attempt.

**S10 — OAuth fails (user denied in browser, CLI exits non-zero)**
- Preconditions: in `waiting`, user clicked Deny in OAuth consent screen.
- Steps:
  1. CLI exits with status != 0. Backend transitions to `Failed { error: "<cli stderr>" }`.
  2. State → `failed`. Same UI as S9.

**S11 — auth.start RPC fails (CLI not found, npm install failed)**
- Preconditions: `resolvecli` failed during the modal's `startConnect` helper.
- Steps:
  1. `controller.failConnect(error)` synthesizes a `failed` state with the real error message.
  2. FailedBanner shows the ResolveCli error (not a generic "CLI not found at ''" — that was the #847 reagent finding).

**S12 — User selects a non-blank bundle but it's missing the provider account**
- Preconditions: bundle "Codex (asaf@x.com)" has Codex binding only. User picked it but is launching the Claude agent.
- Steps:
  1. Reducer sees `outcome = needs-account` (computed by view from `listidentitybindings`).
  2. State → `unauthenticated`. CTA reads **"Connect Claude to this bundle"** (not "create a new bundle").
  3. Click Connect → `auth.start` with `into_bundle_id = "bundle-N"` (existing bundle id).
  4. CLI auth flow same as S1, but on Save: backend inserts a new `db_identity_accounts` row + binding under the **existing** bundle. No new `db_identities` row.
  5. State → `ready { bundleId: "bundle-N" }`. Launch = E.

**S13 — Bundle creds are stale (refresh-token TTL exceeded)**
- Preconditions: selected bundle has Claude account but `last_refreshed_at_ms` > 90d.
- Steps:
  1. Outcome from view = `expired`. State → `expired`.
  2. CTA reads **"Re-authenticate this bundle"**. Launch = D.
  3. Click → same Connect flow with `into_bundle_id = current`. No new bundle; account row gets fresh `last_refreshed_at_ms`.
  4. State → `ready`.

**S14 — User edits the suggested name in SaveBundle then clicks Save**
- Preconditions: in `authenticated { email }`, name input prefilled.
- Steps:
  1. User edits name (e.g. "Work Claude").
  2. Click Save → state → `saving`. RPC `auth.savebundle { session_id, bundle_name }` fires.
  3. On success → `ready`. On failure (DB error, name conflict) → `failed { error }`.

**S15 — Name input left empty**
- Preconditions: in `authenticated`. User clears the input.
- Steps:
  1. Save button disabled while `name.trim() === ""`. No state change.
  2. Save can only be clicked with a non-empty name.

**S16 — Two concurrent OAuth sessions in different agent panes**
- Preconditions: user opens Launch modals for two different agents in two tabs simultaneously, both pick blank, both click Connect.
- Steps:
  1. Each modal owns its own `AuthFlowController` instance with its own session id.
  2. Backend's `AuthSessionManager` is a per-process singleton; both sessions coexist in its `HashMap`.
  3. Polls are independent. Cancel / Save operate on their own session id.
  4. No cross-talk.

**S17 — Renderer crash mid-OAuth**
- Preconditions: in `waiting`. Renderer OOMs.
- Steps:
  1. CEF restarts the renderer (or the user reloads). PreLaunchAuthPanel re-mounts fresh.
  2. The backend session is still running (its tokio task is independent).
  3. On re-mount the modal has no session id — it's effectively starting from scratch.
  4. Stale backend session times out at 10 min, kills its CLI.
  5. **Open question:** should we persist session-id across renderer crashes? For Phase 1, no — the user just clicks Connect again. Mark as P2 follow-up.

**S18 — User pastes a callback URL after browser-didn't-open**
- Preconditions: in `waiting`. The auth URL was captured but browser didn't open.
- Steps:
  1. URL panel shows the auth URL + Copy button + "paste back URL" input.
  2. User copies URL, opens it manually in a phone / different machine, authorizes, gets a redirect URL like `https://claude.ai/oauth/callback?code=...`.
  3. User pastes redirect URL into the modal input, clicks Submit.
  4. Controller fires `auth.submitcallback { session_id, callback_url }`. Backend writes URL to CLI stdin.
  5. CLI completes the exchange. Same Success path as S1.

**S19 — Device-flow provider (Copilot)**
- Preconditions: provider authType=`oauth` but uses device flow. Blank selected.
- Steps:
  1. Click Connect → `waiting`, CLI emits "Enter code XXXX-YYYY at https://github.com/login/device".
  2. Backend's pattern matcher emits `CodeEmitted { code, verification_url }`.
  3. Frontend state captures `deviceCode = { code, verificationUrl }`. UI shows big code + verify URL link.
  4. User enters code on any device. CLI completes the device-flow exchange.
  5. Same Authenticated → SaveBundle → Ready path as S1.

### Combined / edge stories

**S20 — User in SaveBundle panel changes the identity dropdown**
- Preconditions: in `authenticated { email }`, name input partially filled. User changes dropdown to a different (existing) bundle.
- Steps:
  1. State → `Selected` command. Reducer transitions out of `authenticated` based on new outcome.
  2. **The captured email/account is discarded.** Open question: should we offer "use this auth to add an account to <other-bundle>"? For Phase 1, no — too confusing. Discarded.
  3. The not-yet-saved backend session is cancelled. CLI auth result is lost. (User has to re-Connect to use it.)

**S21 — User in SaveBundle clicks Cancel**
- Preconditions: in `authenticated`.
- Steps:
  1. Add a Cancel button to the SaveBundle panel.
  2. Click → state → `unauthenticated`. Backend session cancelled, captured creds discarded.
  3. Returns to Connect CTA.

**S22 — Save RPC fails with name collision**
- Preconditions: in `saving`. User chose a name that already exists.
- Steps:
  1. Backend transaction fails (UNIQUE constraint? or our own check?). Returns `Err`.
  2. Controller dispatches transition back to `authenticated` (preserve email), surfaces error in a banner above the name input.
  3. User can edit name + retry.
  - **Note:** check if `db_identities.name` has UNIQUE constraint. If not, dupes are allowed — no collision possible. (Verify before implementing.)

---

## 4. State enumeration

`AuthState.kind` becomes 9 variants (4 new):

| Kind | Meaning | Launch button |
|---|---|---|
| `idle` | Initial — selection hasn't fired yet | D |
| `ready` | Valid bundle bound for the selected provider, fresh | **E** |
| `expired` | Bundle has provider account but creds stale | D |
| `unauthenticated` | Bundle is blank OR missing provider account | D |
| `waiting` | OAuth/api-key RPC in flight, CLI running | D |
| `authenticated` | CLI auth done, awaiting save (NEW) | D |
| `saving` | Save RPC in flight (NEW) | D |
| `failed` | Any terminal failure | D |
| `(closed=true flag)` | Modal/pane disposed; all commands no-op |  D |

New fields on `AuthState`:
- `email: string` (populated when `kind === "authenticated" | "ready"` and OAuth source)
- `suggestedName: string` (the prefilled default; reducer computes this on enter to authenticated)
- `intoBundleId: string` (when the connect was a "re-auth" or "add account" flow, the existing bundle to update instead of insert-new)

---

## 5. State diagram

```
                                ┌─────┐
                                │idle │  (initial)
                                └──┬──┘
                                   │ Selected
                                   ▼
                ┌──────────────────┴──────────────────┐
                │                                     │
   outcome=ready│outcome=expired   outcome=needs-account/needs-bundle
                │      │                              │
                ▼      ▼                              ▼
            ┌─────┐ ┌───────┐                   ┌──────────────┐
            │ready│ │expired│                   │unauthenticated│
            └─────┘ └───┬───┘                   └───────┬───────┘
                        │ ConnectClicked                │ ConnectClicked
                        │ (re-auth path)                │ (new-account path)
                        └────────────┬──────────────────┘
                                     │
                                     ▼
                                ┌────────┐
                       ┌────────│waiting │◄────────┐
                       │        └───┬────┘         │
            CancelClicked            │ Polled.success / ApiKeyAccepted (with email)
                       │             │
                       ▼             ▼
                ┌──────────────┐  ┌──────────────┐
                │unauthenticated│  │authenticated │
                └──────────────┘  └──┬───────────┘
                                     │ SaveBundleClicked
                                     ▼
                                ┌────────┐
                                │ saving │
                                └───┬────┘
                                    │ BundleSaved (success)
                                    │   OR  BundleSaveFailed
                                    │
                            ┌───────┴───────┐
                            ▼               ▼
                         ┌─────┐      ┌──────────────┐
                         │ready│      │authenticated │
                         └─────┘      │ (error shown)│
                                       └──────────────┘

  Any state ──Polled.failed──▶ failed ──ConnectClicked──▶ waiting (retry)
  Any state ──Disposed──▶ closed=true (terminal)
```

---

## 6. Commands (new + revised)

| Command | New? | Allowed from | Transition |
|---|---|---|---|
| `Selected { providerId, bundleId, outcome, intoBundleId? }` | revised | any | → idle / ready / expired / unauthenticated per outcome; clear all session state |
| `ConnectClicked` | unchanged | unauthenticated, expired, failed | → waiting; carry `intoBundleId` from current state |
| `SessionStarted { sessionId, authUrl? }` | unchanged | waiting | record sessionId; stay waiting |
| `Polled { sessionId, status }` | revised | waiting | status=success → **authenticated** (was → ready) |
| `CancelClicked` | unchanged | waiting | → unauthenticated, fire backend cancel |
| `CallbackSubmitted` | unchanged | waiting (with sessionId) | event only |
| `ApiKeySubmitted` | revised | unauthenticated, expired, failed | → waiting (api-key path) |
| `ApiKeyAccepted { email }` | revised | waiting | → **authenticated { email }** (was → ready) |
| `SaveBundleClicked { name }` | **NEW** | authenticated | → saving |
| `BundleSaved { bundleId }` | **NEW** | saving | → ready { bundleId } |
| `BundleSaveFailed { error }` | **NEW** | saving | → authenticated (preserve email + suggestedName), error in event |
| `Disposed` | unchanged | any | closed=true; idempotent |

---

## 7. Events emitted

In addition to existing events:
- `authenticated { email }` — fired when entering authenticated state (controller side-effects: focus name input)
- `save-requested { sessionId, name }` — fires `auth.savebundle` RPC
- `bundle-saved { bundleId, name }` — fires `props.onBundleCreated` in panel
- `save-failed { error }` — surface inline

---

## 8. Backend wire mapping

Backend `AuthSessionStatus` (Rust enum) needs one new variant:

```rust
pub enum AuthSessionStatus {
    Pending,
    UrlAvailable { auth_url: String },
    CodeEmitted { device_code: String, verification_url: String },
    Authenticated { email: Option<String> },   // NEW — was the only success state
    Success { bundle_id: String, email: Option<String> },  // NOW: post-save
    Failed { error: String },
}
```

Mapping wire → reducer command:
- wire `authenticated` → reducer `Polled { status: { status: "authenticated", email } }` → kind = authenticated
- wire `success` → reducer `Polled { status: { status: "success", bundleId, email } }` → kind = ready (only reachable AFTER savebundle commits)

New RPC: `auth.savebundle { sessionId, bundleName } -> { bundleId } | error`.

New mgr method `commit_bundle(sid, name) -> Result<bundle_id, String>` does the 3-row transaction:
1. Look up captured email + provider from session
2. If session has `into_bundle_id`: use existing bundle_id (skip the `db_identities` insert; only insert/upsert the account + binding for this provider)
3. Else: generate new bundle uuid; insert `db_identities` row with name
4. Generate account uuid; insert `db_identity_accounts` row with the provider, kind="oauth", display_name=email, secret_ref pointing at the per-provider auth dir
5. Upsert `db_identity_bindings` row (identity_id, provider, account_id)
6. Transition session status to `Success { bundle_id, email }`

Transactionally — wrap in rusqlite `BEGIN / COMMIT / ROLLBACK on error`.

---

## 9. Invariants

The reducer must enforce:

1. **Post-close gate.** `closed=true` → every command except `Disposed` returns the state unchanged + emits `post-close-command-dropped`.
2. **Session-id match.** `Polled` and `CallbackSubmitted` drop unless `command.sessionId === state.sessionId`.
3. **Kind guard on `SessionStarted` / `ApiKeyAccepted`.** Only honored while `kind === "waiting"` — prevents zombie sessions after Cancel.
4. **`SaveBundleClicked` requires `kind === "authenticated"`.** Drop otherwise.
5. **`BundleSaved` / `BundleSaveFailed` require `kind === "saving"`.** Drop otherwise.
6. **`Selected` clears every transient.** sessionId, email, suggestedName, deviceCode, authUrl, error — all reset. Stops any pending poll. Fires `auth.cancel` for the prior session (controller side-effect).
7. **Idempotent `Selected`.** Same `(providerId, bundleId, outcome)` triple → no-op (prevents the createEffect re-fire bug).
8. **`Disposed` is terminal + idempotent.**

---

## 10. Test surface (additions)

New reducer tests (slice #11):
- `RunStarted` → kind=authenticated with email + suggestedName populated
- `SaveBundleClicked` from authenticated → kind=saving
- `BundleSaved` from saving → kind=ready, bundleId set
- `BundleSaveFailed` from saving → kind=authenticated (preserve email), error in event
- `SaveBundleClicked` from non-authenticated → dropped
- `BundleSaved` from non-saving → dropped
- `Polled.authenticated` (new wire variant) → kind=authenticated
- `Selected` mid-saving → cancels the save, resets per outcome
- `expired → ConnectClicked` → waiting with intoBundleId carried
- All existing tests stay green

Controller tests:
- `saveBundle(name)` fires `auth.savebundle` RPC, transitions through saving
- Save RPC error transitions to authenticated with error
- Stale Polled (sessionId mismatch) still dropped
- Cancel during authenticated also fires backend cancel (the session is still alive backend-side until committed)

---

## 11. Migration plan

PR ordering, all stacked on `agenta/oauth-prelaunch-modal`:

1. **PR C-1 — Reducer extension.** New states + commands + tests. Pure frontend, no RPC changes. Reducer LGTM gates merging.
2. **PR C-2 — Backend Authenticated state + commit_bundle + savebundle RPC.** Backend Rust changes. Unit tests for commit_bundle (3-row transaction).
3. **PR C-3 — Controller `saveBundle` method + adapter for new RPC.** Wires the two halves. Adds new event sink.
4. **PR C-4 — UI SaveBundle panel.** Renders for `kind === "authenticated"`; name input prefilled with `suggestedName`; Save button calls controller. Smoke-test gate before merge.
5. **PR C-5 — needs-account outcome computation in modal.** View calls `listidentitybindings` per non-blank bundle, computes `needs-account` vs `ready`. Now S12 actually works.
6. **PR C-6 — expired outcome from stale `last_refreshed_at_ms`.** S13 wired. Requires the storage spec's last_refreshed_at_ms column.

PRs C-1..C-4 are the user-visible "name and save" loop. C-5 + C-6 are the per-bundle gating that makes mixed-provider bundles work.

---

## 12. Open questions

1. **S20 — bundle-swap during SaveBundle.** Should we preserve the captured creds and let the user pick a different bundle to attach them to? Or always discard? Spec says discard; revisit if users actually try this.
2. **S22 — name collision.** Are duplicate `db_identities.name` allowed? If not, where's the constraint enforced (DB UNIQUE vs handler check)? **TODO: verify before C-2.**
3. **S17 — renderer-crash session recovery.** Spec says no, but if users hit this in practice (especially with the OOM issues we've seen), revisit.
4. **Per-provider TTL table.** Anthropic ~90d, OpenAI ~30d, Google ~30d, GitHub effectively forever. S13 needs this table loaded somewhere — `providers/index.ts` is the natural home.
5. **Where does the secret_ref point?** For OAuth providers we capture creds into `<auth-config-dir>/<provider>/...` via env vars at spawn time. The bundle's `SecretRef::PlaintextDev { plaintext_dev: <dir-path> }` is a placeholder; PR 2 of the storage spec encrypts via keyring. Until then we ship PlaintextDev with a `dev_only=true` marker, fail loudly in release builds.
6. **Modal close without explicit Cancel.** `dispose()` already fires `auth.cancel` (PR B-2 fix). The same path needs to apply if `kind === "authenticated"` (CLI is done but session is still kept alive backend-side waiting for savebundle). Add cancel on dispose for authenticated too.

---

## 13. Acceptance criteria

From the user's exact ask, plus the standard spec criteria:

- [ ] No bundles exist + Blank selected: Connect CTA shows. Launch disabled.
- [ ] OAuth succeeds: SaveBundle panel appears with default name `<DisplayName> (<email>)`. Launch still disabled.
- [ ] User clicks Save with default or edited name: bundle row + account row + binding row created in one transaction. Dropdown refetches and shows the new bundle. Bundle is auto-selected. Launch enables.
- [ ] User edits the name to "" → Save button disabled.
- [ ] User clicks Cancel from SaveBundle: backend session cancelled, captured creds discarded, returns to Connect CTA.
- [ ] User selects an existing valid bundle: Launch enables immediately. No Connect CTA.
- [ ] User selects an existing bundle missing the provider's account: "Connect <provider> to this bundle" CTA. On Connect → SaveBundle skips the name input (we already have a bundle name) — clicking Save just adds the account + binding rows under the existing bundle.
- [ ] User selects an expired bundle: "Re-authenticate" CTA. On Connect → SaveBundle skips the name input. Account row updated, binding unchanged.
- [ ] User cancels mid-OAuth: backend session killed, no orphan CLI.
- [ ] User closes modal mid-OAuth: same as Cancel (`dispose()` path).
- [ ] User changes identity dropdown mid-OAuth: previous session cancelled, state recomputes per new outcome.
- [ ] Two modals in two panes, both authing simultaneously: independent sessions, no cross-talk.
- [ ] On terminal `failed` state: retry CTA. Click retry → back to waiting.
- [ ] API-key providers: paste panel instead of OAuth flow. Save commits the api-key bundle.
- [ ] Continue-agent path bypasses the entire gate.

---

## 14. References

- Parent: [`SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md`](SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md) — UX + backend RPCs
- Companion: [`SPEC_OAUTH_IN_IDENTITY_BUNDLES_2026_05_13.md`](SPEC_OAUTH_IN_IDENTITY_BUNDLES_2026_05_13.md) — token persistence/refresh
- Diagnostic: [`OAUTH_FLOW_SMOKE_DIAGNOSTIC_2026_05_14.md`](OAUTH_FLOW_SMOKE_DIAGNOSTIC_2026_05_14.md) — root cause of the dispatch re-fire bug
- Resume notes: [`OAUTH_RESUME_AFTER_REBOOT_2026_05_14.md`](OAUTH_RESUME_AFTER_REBOOT_2026_05_14.md) — what's uncommitted on disk
- Reducer roadmap: master reducer-stack status doc (slice #11 = workflow-run-state; new slice #12 candidate for this work, or we extend the existing PR B-1 slice in place)
- Existing reducer: `frontend/app/view/agent/auth/auth-state.ts` (PR B-1, merged via #849)
- Existing controller: `frontend/app/view/agent/auth/auth-flow-controller.ts` (PR B-2, in #850)
