# SPEC: Block identity bindings from resolving to a provider's ambient home dir

**Date:** 2026-08-25
**Status:** implemented.

---

## 0. Ask

Follow-up to a direct question ("do you have evidence that an agent inside
AgentMux will never read the `~/.claude`? or write to it?") that surfaced a
real, previously unvalidated gap — see §1. The user's direction, verbatim:

> so can we enforce that agents never use the ~/.claude?

Followed by a scoping decision on the one genuinely disruptive part (how to
treat an already-live account configured this way): **"Block it too, fail
loudly"** — symmetric enforcement, no silent grandfathering. And:

> AgentY and Lark should be blocked from using it, this is a good test in
> case it ever reverts, when I open AgentY I want a graceful recovery

Two requirements: (1) block unconditionally, including existing bindings,
and (2) opening a blocked agent must show a clear, actionable message —
not a dead-end "Retry" (the exact failure mode a prior incident already
fixed once for a different gate refusal, see §3.3).

---

## 1. The gap (audit findings)

A dispatched audit (full report not reproduced here) found:

- The **primary coding-agent spawn path** (`agent_open.rs` → `input.rs`/
  `agent_io.rs` → `blockcontroller/persistent.rs`) has no reachable
  "`CLAUDE_CONFIG_DIR` silently unset" gap in production code — confirmed
  clean.
- **No code anywhere writes to `~/.claude/CLAUDE.md` directly** — the two
  Global Memory display RPCs (`getclaudeglobalconfig`/`getclaudehostconfig`,
  `agent_handlers/memory.rs`) are genuinely read-only.
- **But the identity-bound spawn gate (`identity/resolver/inject.rs`)
  injects `CLAUDE_CONFIG_DIR` = whatever `secret_ref.dir` says in the
  database, with zero validation that the value is inside AgentMux's own
  isolated tree.** The `upsertidentityaccount` RPC
  (`agent_handlers/identity.rs`) persisted `secret_ref` verbatim, with no
  containment check — unlike `auth.start`'s own account-creation path,
  which always mints an isolated dir.
- **This is not hypothetical.** `docs/status/STATUS_IDENTITY_ISOLATION_GATE_NOT_ENFORCING_2026_08_20.md`
  §8 found a real, currently-linked account (`secret_ref.dir` = the
  literal ambient home) bound to two live agents. Independently, querying
  *this* machine's own live `~/.agentmux/shared/identity-store.db` during
  this spec's own investigation found the same class of live account:
  `id=b3a58e33-97e1-492a-af30-a26973ec855e`, `secret_ref: {"backend":
  "oauth_config_dir","dir":"C:\Users\area54\.claude"}` — the literal
  ambient path on this host. Since `CLAUDE_CONFIG_DIR` relocates Claude
  Code's *entire* home (`SPEC_PROVIDER_ISOLATION_2026_06_20.md` §5b), any
  agent spawned against that account reads and writes
  `~/.claude/CLAUDE.md`, exactly the file the operator asked to keep
  agents out of.
- A separate, earlier spec (`SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md`
  §7.2) had claimed a *different* mechanism (working-directory `CLAUDE.md`
  composition) explained an earlier "ambient content leaked into an
  agent's context" observation. The audit flags that explanation as an
  unverified non-sequitur — this account-binding gap is the far more
  direct, actually-evidenced explanation. Not corrected in that spec's own
  text (out of scope here — flagged for a future pass on that doc).

---

## 2. Design — symmetric enforcement, two layers + one classifier

### 2.1 Shared detection helper (`backend/providers.rs`)

```rust
pub fn is_provider_ambient_home_dir(provider: &ProviderConfig, dir: &str) -> bool {
    let ambient = get_home_dir().join(format!(".{}", provider.auth_dir_name));
    paths_resolve_to_same_dir(&ambient, dir)
}
```

`provider.auth_dir_name` already gives the exact ambient-home suffix for
every registered provider (`"claude"` → `~/.claude`, `"codex"` → `~/.codex`,
etc. — the same field `provider_auth_dir()` uses for the isolated-dir
side). `paths_resolve_to_same_dir` is split out with the reference path
pre-resolved (not calling `get_home_dir()` internally) specifically so
it's directly testable against a tempdir, same "inject the path, don't
resolve it internally" pattern `read_claude_global_config` already uses.
Canonicalizes both sides when possible (defeats `..`/symlink/`\\?\`-prefix
tricks, same approach `identity::cleanup`'s containment check already
uses); falls back to a normalized (lowercase, `/`-separator, no trailing
slash) lexical comparison when either side doesn't exist yet on disk, so a
not-yet-materialized ambient path can't slip past the guard just by not
existing at check time.

### 2.2 Write-time guard — `upsertidentityaccount` (`agent_handlers/identity.rs`)

New `reject_ambient_home_dir_binding(&IdentityAccount) -> Result<(), String>`,
called before persisting. Refuses (no write happens) whenever
`secret_ref` is `OAuthConfigDir { dir }` and `dir` resolves to that
provider's ambient home. Split out of the handler closure specifically so
it's unit-testable without spinning up the RPC engine.

### 2.3 Spawn-time guard — `inject_identity_env_with_broker` (`identity/resolver/inject.rs`)

Checked right after resolving `dir` from the bound account's
`SecretRef::OAuthConfigDir`, before it's inserted into `env_vars`. On a
match, returns a new error variant unconditionally — **not** gated by
`use_ambient_login` (that escape hatch was already retired for the
sibling `MissingCredentials` case; this is a new, always-on check, same
posture). This is the layer that blocks the *existing* live account too —
per the user's explicit choice (§0), there is no grandfathering.

```rust
if let Some(provider_cfg) = get_provider(&resolve_provider_alias(&binding.provider)) {
    if is_provider_ambient_home_dir(provider_cfg, &dir) {
        return Err(SpawnGateError::AmbientHomeDirNotAllowed {
            provider: binding.provider.clone(),
            dir: dir.clone(),
        });
    }
}
```

### 2.4 New `SpawnGateError` variant + Display wording (`identity/resolver/errors.rs`)

```rust
AmbientHomeDirNotAllowed { provider: String, dir: String },
```

Display: *"this agent's {provider} identity points directly at your
personal {provider} config directory ({dir}) instead of an isolated
AgentMux account — AgentMux no longer allows spawning an agent against
your own global CLI login. Re-bind this identity to an isolated account
in Armory → Accounts (delete the current {provider} account and log in
again to create a fresh, isolated one), then retry."* — same "spawn
callers surface `Display` verbatim in the agent pane" mechanism the
sibling `MissingCredentials` variant already relies on
(`error_during_execution` frame).

### 2.5 Graceful recovery — failure classifier (`agents/failure.rs`)

Without a matching classifier branch, this error's raw text would fall
through to `FailureClass::UnknownNonZero`, which the codebase's own prior
incident (`retro-agentu-0.54.9-stuck-error-2026-08-03.md`, cited inline
next to the sibling `MissingCredentials` branch) documents as a dead end:
"a retry can never succeed against a gate that blocks every respawn
identically." Added a parallel branch, matched on a stable substring of
the Display wording (`"instead of an isolated agentmux account"`,
lowercased-`hay` match same as the existing branch):

- `FailureClass::Auth`, `retryable: false`
- Title: *"Identity points at your personal login"*
- Detail names the actual provider (extracted from the gate's own wording
  via `extract_ambient_home_provider`, mirroring
  `extract_spawn_gate_provider`'s "never guess, fall back to generic
  phrasing on wording mismatch" contract) and points at Armory → Accounts.

This is what makes opening a blocked agent (e.g. one bound to the live
account found in §1) show a clear, actionable card instead of a stuck
"Retry" button — the "graceful recovery" from §0.

---

## 3. Out of scope

- **Migrating the existing live account(s)** — the user's explicit choice
  (§0) was to block, not auto-migrate. An operator hitting this now sees
  the graceful error and re-binds manually via Armory → Accounts.
- **Validating other `SecretRef` variants** (`Env`, `SecretsManager`,
  `PlaintextDev`, `Keychain`) — this gap is specific to
  `OAuthConfigDir`'s filesystem-pointer shape; the others don't carry a
  raw directory path at all.
- **A broader "any dir outside AgentMux's tree" containment check** — the
  ask was specifically "never use `~/.claude`" (the literal ambient home),
  not "must be inside `~/.agentmux/`". A user-supplied custom dir that's
  neither the ambient home nor AgentMux-managed is a different, narrower
  question not raised here.
- **Correcting `SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md` §7.2's
  likely-incorrect explanation** — flagged in §1, left as a follow-up on
  that doc specifically.
- **Container agents** — structurally separate filesystem (Docker);
  `CLAUDE_CONFIG_DIR`/`HOME`/`USERPROFILE` are already denylisted from
  being forwarded in (`backend/container.rs`'s `CONTAINER_ENV_DENYLIST`).
  Not touched by or relevant to this change.

---

## 4. Test plan

**`backend/providers.rs`** (`ambient_home_dir_tests` module):
- [x] Identical existing dirs match via canonicalize.
- [x] Different existing dirs do not match.
- [x] Nonexistent paths on both sides fall back to lexical comparison and
      still correctly match/mismatch (confirms a not-yet-created ambient
      path can't slip past by not existing at check time).
- [x] Lexical fallback is case-insensitive and ignores trailing
      slash/separator style (`/` vs `\`).
- [x] `is_provider_ambient_home_dir`'s public wrapper joins
      `get_home_dir()` with `.{auth_dir_name}` as documented.

**`identity/resolver/inject.rs`**:
- [x] `inject_oauth_class_blocks_spawn_when_config_dir_is_the_ambient_home`
      — uses the REAL `get_home_dir()` (not a mock) to construct the
      account's `dir`, so this exercises the exact resolution production
      code does. Asserts `Err(SpawnGateError::AmbientHomeDirNotAllowed)`
      with the correct provider/dir, and that no partial env-var leak
      happens before the gate fires.
- [x] Existing `inject_oauth_class_sets_config_dir_env_var` (isolated dir)
      re-verified still passes — the guard doesn't false-positive on a
      normal, isolated account.

**`agent_handlers/identity.rs`** (`ambient_home_dir_binding_tests` module):
- [x] Refuses an `OAuthConfigDir` pointed at the real ambient home
      (same real-`get_home_dir()` approach as the inject.rs test).
- [x] Allows an `OAuthConfigDir` pointed at an isolated dir.
- [x] Allows non-OAuth `SecretRef` variants unconditionally.
- [x] Allows an unrecognized provider id unconditionally (nothing to
      check against — matches existing "unknown provider" skip behavior
      elsewhere, not a new failure mode).

**`agents/failure.rs`**:
- [x] The gate's `AmbientHomeDirNotAllowed` refusal classifies as
      `FailureClass::Auth`, `retryable: false`, not `UnknownNonZero`.
- [x] Provider name in the detail matches the actual failing provider
      (Codex case doesn't say "Claude" and vice versa).
- [x] `extract_ambient_home_provider` reads the provider out of the gate's
      own wording; returns `None` on wording mismatch rather than
      guessing.

**Manual / live-data verification (done during this spec's own
investigation, not a repeatable automated test):**
- [x] Queried this host's live `~/.agentmux/shared/identity-store.db`
      directly and confirmed a real account (`b3a58e33-...`) has
      `secret_ref.dir` = the literal ambient home
      (`C:\Users\area54\.claude`) — proving this isn't a hypothetical
      gap on this system. Did not find this specific account linked to
      a named agent via `db_agent_identity_links` at query time (zero
      rows for that `account_id`) — whether the user's named "AgentY"/
      "Lark" agents specifically resolve to this exact account, a
      different one shaped the same way, or live in a different
      environment than this host was not conclusively traced; the fix
      is verified against the exact real-world shape of the bug via the
      unit tests above regardless.

**Full suite:** `cargo test --bin agentmux-srv` run after all changes —
2813 passed, 0 failed.

---

## 5. Codex review round (PR #2802) — three real, confirmed findings, all fixed

**P1 — the "graceful recovery" classifier was dead code for the actual
production path.** Verified directly: `agent_handlers/input.rs` and
`app_api/agent_io.rs` (the two real pre-spawn `SpawnGateError` call sites)
build a raw `error_during_execution` frame and return early on the gate's
`Err` — neither ever calls `classify()`. Every production `classify()`
call site (`agents/runner.rs`, `blockcontroller/health.rs`,
`blockcontroller/persistent.rs`, `subprocess/host_spawn.rs`) is POST-spawn
(a real process exited or a `HealthMonitor` reclassified an in-band
error) — none apply when the spawn was refused before any process
existed. The frontend's actual recovery-card mechanism is the structured
`agent:last_failure` block-meta key (written by
`blockcontroller::core::persist_last_failure`) plus the ephemeral
`EVENT_AGENT_FAILURE` WPS push — not anything derived from the raw
persisted frame. So §2.5's classifier addition, while itself correct, was
never actually invoked for this feature's own headline UX requirement
("graceful recovery" — §0). **Fix:** both call sites now classify the
gate's `Display` text (`classify(None, None, &gate.to_string(), None)`,
same shape `health.rs`'s in-band-error reclassification uses), then
`persist_last_failure` + publish `EVENT_AGENT_FAILURE`, mirroring
`host_spawn.rs`'s exact post-exit publish sequence.

**P1 — dot-segment bypass in the lexical fallback.** `paths_resolve_to_same_dir`'s
fallback (used whenever either side doesn't exist on disk yet — the
common case for validating a *new* binding before anything is created)
did plain string comparison after case-folding, with no `.`/`..`
collapsing. A binding of `$HOME/.claude/.` (or any `..`-containing
equivalent) compared unequal to `$HOME/.claude` under that fallback,
passing both guards on a fresh machine — then, once actually used,
resolves to the literal ambient dir anyway. **Fix:**
`normalize_path_lexically` now walks `Path::components()`, collapsing
`CurDir` and popping `ParentDir` against the preceding `Normal` segment
(never past a root/prefix), before the (still fallback-only) case-fold
step.

**P2 — case-folding must be platform-conditional.** The fallback
unconditionally lowercased both sides. On a case-sensitive filesystem
(Linux ext4), a not-yet-created `$HOME/.CLAUDE` would incorrectly compare
equal to `$HOME/.claude` while neither existed, then — once both existed —
`canonicalize` would correctly tell them apart, making the guard's
verdict depend on creation order instead of being deterministic. **Fix:**
case-fold only when `cfg!(target_os = "windows") || cfg!(target_os =
"macos")`.

**P2 — `muxspect`'s diagnostic classifier didn't recognize the new
wording.** `classify_last_error_source` (`muxspect_handlers.rs`) only
matched messages starting with `"no credentials for"` or `"credential
injection could not run"` — every `AmbientHomeDirNotAllowed` refusal
(starts with `"this agent's"`) reported source `unknown` instead of
`identity`, regressing this diagnostic specifically for pre-spawn
refusals. **Fix:** added the third prefix to the classifier's condition.

**Test plan (additive, all passing):**
- [x] `providers.rs`: `lexical_fallback_collapses_dot_and_dotdot_segments`
      (the P1 dot-segment fix, including a genuinely-different-directory
      negative case reached via `..`), `lexical_fallback_case_folding_matches_platform_default`
      (asserts whichever behavior is correct for whatever OS actually runs
      the test, so it's meaningful on both CI legs).
- [x] `muxspect_handlers.rs`: `classify_last_error_source_matches_every_known_construction_site`
      extended with the `AmbientHomeDirNotAllowed` wording.
- [x] Manual trace (not a new automated test — verified by reading the
      call graph): confirmed `classify()` has no other production call
      site that would have already covered the pre-spawn path, ruling out
      "already fixed elsewhere" before writing the `input.rs`/`agent_io.rs`
      fix.
