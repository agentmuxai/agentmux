# OAuth pre-launch — resume notes after PC reboot (2026-05-14)

## Open work on disk (not pushed)

Branch: `agenta/oauth-prelaunch-modal` (PR #847)

Uncommitted changes:
- `frontend/app/view/agent/auth/auth-flow-controller.ts`
  - Diagnostic `console.log("[auth-diag] …")` calls throughout (selected, connect, schedulePoll, pollOnce, dispose)
  - `dispatch()` now wrapped in `untrack()` — defense-in-depth so callers in a reactive scope don't auto-subscribe to `_state` via dispatch's read
- `frontend/app/view/agent/components/PreLaunchAuthPanel.tsx`
  - Diagnostic `console.log` in PreLaunchAuthPanel MOUNT, selection-effect, dispose
  - **Root-cause fix:** the selection `createEffect` now wraps `controller.selected(…)` in `untrack()` — breaks the dispatch → re-fire loop that was wiping in-flight sessions
- `agentmux-srv/src/server/identity_handlers.rs`
  - Diagnostic `tracing::info!` in `auth.spawn` (Command::spawn entry/result), stdout/stderr drains, child.wait()
  - Diagnostic on the `auth.poll` handler

## What the diagnostic logs proved

Last run captured this sequence (in `[fe] [auth-diag]` lines):
```
selected called, kind_before=unauthenticated → dispatched, kind_after=unauthenticated
selected called AGAIN, kind_before=unauthenticated (same!) ← the loop
```

Both fires came from the same `createEffect`. Root cause: `controller.selected` calls
`controller.dispatch` which reads `_state` to fold the reducer. That read added
`_state` as a tracked dep of the calling effect. Every subsequent dispatch (from
`ConnectClicked`, `SessionStarted`, etc.) re-fired the same effect, which called
`selected` again, which called `stopPolling()` and dispatched another `Selected` —
wiping the in-flight sessionId before `auth.poll` could fire even once.

The `untrack` fix breaks the chain. Frontend HMR'd the fix in but the renderer OOM'd
before user could re-test.

## What's still TODO

### Immediate (after reboot)

1. **Smoke-test the `untrack` fix** — restart `task dev`, click Connect, verify in the
   `[fe] [auth-diag]` lines that `selected` fires **once** per click and `pollOnce`
   actually reaches `awaiting rpc.poll`. The backend `auth.poll` RPC log should appear
   for the first time.

2. **Remove the diagnostic logs** once the fix is confirmed working. Keep the `untrack`
   in `dispatch` and in the selection effect — that's the actual fix.

### Then — PR C (real bundle auto-creation)

User asked for: "if the state starts with a blank identity, after login give the
user a chance to save the identity name." Two-phase commit required.

**Backend changes:**

1. Add a new `AuthSessionStatus::Authenticated { email }` between `UrlAvailable`/
   `CodeEmitted` and `Success`. Transition there when the CLI confirms login but
   before any DB row is persisted.

2. New mgr method `commit_bundle(session_id, bundle_name) -> Result<bundle_id, String>`:
   - Verifies session is in `Authenticated` state
   - Generates uuid for bundle_id + account_id
   - Creates `db_identities` row (`name = <bundle_name>`)
   - Creates `db_identity_accounts` row (provider, kind="oauth",
     display_name=email, secret_ref=`PlaintextDev { plaintext_dev: <auth-config-dir> }`)
   - Creates `db_identity_bindings` row (identity_id, provider, account_id)
   - All three in one rusqlite transaction
   - Transitions session to `Success { bundle_id, email }`

3. New RPC handler `auth.savebundle { session_id, bundle_name } -> { bundle_id }`.

4. Update `spawn_auth_cli` success paths (stdout LoginSuccess + post-exit confirm)
   to call `mgr.finish_authenticated(sid, email)` instead of `finish_success`.

**Frontend changes:**

5. Add `AuthState.kind = "awaiting-save"` between `waiting` and `ready` (new variant).

6. Reducer's `foldPolled` maps wire `Authenticated` → kind=awaiting-save. Don't
   clear sessionId yet (still need it for the save RPC).

7. New `AuthCommand::SaveBundleClicked { name }`. Controller dispatches it on Save
   click, fires `auth.savebundle` RPC, then dispatches `BundleSaved { bundleId }` →
   transitions to ready with the real id.

8. New UI panel in `PreLaunchAuthPanel`: render between Waiting and Ready. Shows
   "✓ Logged in as `<email>`" + a text input prefilled with `"<ProviderDisplayName> (<email>)"`
   + a Save button. Save → `controller.saveBundle(name)`.

9. The existing `createEffect` that calls `props.onBundleCreated(s.bundleId)` already
   fires when `kind === "ready"` AND `bundleId` is non-empty — works as-is.

### Bundle-side launch wiring (post-PR-C cleanup)

The spawn-time resolver in `agentmux-srv/src/.../resolver.rs` needs to honor the
bundle's account `secret_ref` so a subsequent agent launch finds the same auth dir.
For PR C's MVP, the secret_ref points at the `CLAUDE_CONFIG_DIR`/`CODEX_HOME`/etc.
the auth.start RPC used. The launch flow then sets the same env var when spawning
the agent. **This piece is what makes "log in once, reuse" actually work.**

## Files referenced

- Spec: `docs/specs/SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md` (§10 PR C, §12 acceptance criteria)
- Storage spec: `docs/specs/SPEC_OAUTH_IN_IDENTITY_BUNDLES_2026_05_13.md`
- Diagnostic report: `docs/specs/OAUTH_FLOW_SMOKE_DIAGNOSTIC_2026_05_14.md`
- WStore identity methods: `agentmux-srv/src/backend/storage/wstore.rs:2013` (`bundle_identity_upsert`), `:2052` (`bundle_identity_bind`), `:1460` (`identity_upsert`)
- Auth session manager: `agentmux-srv/src/identity/auth_session.rs`
- Existing handlers: `agentmux-srv/src/server/identity_handlers.rs`

## After-reboot checklist

- [ ] `git status` — confirm the uncommitted changes from this session are still there
- [ ] `task dev` clean start (will recompile srv from scratch if memory is tight; first VITE request takes ~100s)
- [ ] Smoke-test the `untrack` fix
- [ ] Commit the `untrack` + drop the `[auth-diag]` console.logs
- [ ] Start PR C per the plan above
