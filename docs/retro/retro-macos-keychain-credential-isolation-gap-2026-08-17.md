# Retro: macOS Claude Code sessions authenticate with zero linked identity accounts

**Date:** 2026-08-17
**Area:** `agentmux-srv/src/identity/resolver/inject.rs`, `agentmux-cef/src/commands/platform.rs`, macOS Keychain credential storage in the upstream Claude Code CLI

---

## 1. Symptom (as reported)

Inspecting a live, running AgentMux agent instance on macOS: the `IdentityAccounts`
MCP tool returns `{"accounts": []}` — no identity account is bound to this agent in
`db_agent_identity_links` — yet the agent is running normally with full Claude
access. The user's framing: "that would be impossible on Windows."

## 2. Investigation

- Confirmed `db_agent_identity_links` genuinely has no row for this agent
  (`IdentityAccounts` result), ruling out a stale-cache or query bug.
- Read `agentmux-srv/src/identity/resolver/inject.rs`
  (`inject_identity_env_with_broker`, the "layer 3 spawn gate") in full.
  **Correction to an earlier working theory in this investigation:** the
  `use_ambient_login` escape hatch this function's doc comments still
  describe was fully retired — per the "Superseded same-day" note in
  `docs/specs/PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md` §7
  (2026-07-22 correction), a missing bound oauth account now blocks the
  spawn *unconditionally*; the flag is read only for a log line and has no
  effect on the outcome (`spawn_still_blocked_when_bound_oauth_account_missing_and_flag_true`
  pins this directly). So the flag is not what let this agent run — two
  things actually explain it, and neither depends on that flag:
  1. This gate's Step 1 (`instance_get_active_for_block`) returns `Ok(())`
     immediately, with **no gating and no injection at all**, whenever the
     block has no `AgentInstance` row — the function's own comment: "Block
     has no agent instance row — nothing to inject, and no gating either:
     quick-launch panes that never went through the launch modal are
     outside the managed-credentials contract." A pane spawned outside the
     launch-modal flow never reaches the gate in the first place.
  2. Independent of whether the gate fires, the srv-side identity system
     isn't what supplies this agent's credentials anyway — see the next
     bullet. The CEF-host-side shared-provider default (`ensureAuthDir` /
     `ensure_auth_dir`) is a separate, earlier-in-the-pipeline mechanism
     that resolves `CLAUDE_CONFIG_DIR` before/independent of the srv gate,
     confirmed live via 5 current frontend call sites (`agent-model.ts:257,489`,
     `PreLaunchAuthPanel.tsx:429`, `useAgentControllerStatus.ts:256`,
     `ClaudeLoginPanel.tsx:173`) — this is not legacy/dead code, it's the
     active default path today.
- The agent process's actual `CLAUDE_CONFIG_DIR` env var points not at a
  per-identity bundle but at the **legacy shared-provider default path**,
  `~/.agentmux/shared/providers/claude/` — a structurally separate, older
  mechanism documented in `agentmux-cef/src/commands/platform.rs`
  (`ensure_auth_dir`) as the account-wide, version/channel-independent
  default, put in place as the fix for
  `docs/retro/retro-provider-auth-isolation-regression-2026-06-05.md`. The
  layer-3 gate in `inject.rs` only resolves `db_agent_identity_links`
  bindings — it has no knowledge of, and does not gate, this shared-provider
  path at all.
- That shared-provider directory has **no `.credentials.json` file on
  disk.** Checked the macOS Keychain directly (`security dump-keychain` /
  `security find-generic-password`) and found the running session's OAuth
  credential live there instead, under service name `"Claude
  Code-credentials"` (plus several hash-suffixed variants of unclear origin —
  see §5).
- Fetched Claude Code's own authentication docs
  (`code.claude.com/docs/en/authentication`) to get the canonical behavior:
  > - On macOS, credentials are stored in the encrypted macOS Keychain.
  > - On Linux, credentials are stored in `~/.claude/.credentials.json`
  >   (mode `0600`).
  > - On Windows, credentials are stored in
  >   `%USERPROFILE%\.claude\.credentials.json`, ACL-restricted to the user
  >   profile.
  > - **If you've set `CLAUDE_CONFIG_DIR` on Linux or Windows, the
  >   `.credentials.json` file lives under that directory instead.**

  The docs explicitly scope the `CLAUDE_CONFIG_DIR`-relocation behavior to
  Linux and Windows only. macOS is not listed — Keychain storage is not
  redirected by `CLAUDE_CONFIG_DIR` at all.
- Cross-checked against GitHub issue
  [anthropics/claude-code#9403](https://github.com/anthropics/claude-code/issues/9403)
  (a *different* macOS Keychain bug — a service-name read/write mismatch —
  but it incidentally confirms the scoping): "Credentials are **machine-wide**
  in the macOS Keychain (`~/Library/Keychains/login.keychain-db`), stored
  under the user account (`-a "$USER"`)." No per-directory or per-profile
  scoping exists.

## 3. Root cause

AgentMux's whole per-agent/per-identity credential-isolation strategy — for
both the new identity-account system and the legacy shared-provider default —
is built on setting `CLAUDE_CONFIG_DIR` to point each spawned Claude Code
process at a distinct directory. On Linux and Windows this genuinely works,
because the underlying CLI relocates its credential *file* under that
directory. **On macOS it's silently a no-op for credential storage**, because
the CLI's macOS credential storage never consults `CLAUDE_CONFIG_DIR` — it
always reads/writes one Keychain entry (`"Claude Code-credentials"`), scoped
only to the OS user account, machine-wide, regardless of which config
directory the process was launched with.

So on this machine: whatever `CLAUDE_CONFIG_DIR` value happens to be set
(shared-provider default, in this case — but a per-identity bundle dir would
behave identically), the CLI ignores it for auth purposes and transparently
falls through to the single OS-Keychain-resident login that's already
present from some prior `/login` on this Mac. That's why the agent "just
works" with an empty `db_agent_identity_links` binding and no
`.credentials.json` on disk anywhere in its assigned config dir: it was
never actually using its assigned config dir for auth. It's riding the
ambient, account-wide Keychain login.

This also compounds with a documented ACL weakness in how the CLI writes
that Keychain entry: a third-party security disclosure (Silverfort,
"Skipping the lock: A Claude Code CLI weakness lets any macOS process read
stored credentials") found the entry is written via
`/usr/bin/security add-generic-password` without an explicit access-control
list, so its default ACL only names `/usr/bin/security` itself as a trusted
reader — meaning *any* process running as the same OS user, not just Claude
Code, can read the full OAuth token bundle with a single
`security find-generic-password -s "Claude Code-credentials" -w` call, no
prompt. Anthropic's stated position (per that disclosure) is that this is an
accepted design tradeoff, not a vulnerability, since they consider all
same-user processes trusted; they're reportedly tracking a future ACL
tightening as a hardening improvement, not a fix. That "same-user processes
are trusted" assumption doesn't hold inside AgentMux, where separate
identity-scoped agents are supposed to be mutually untrusted with respect to
each other's credentials.

## 4. Why this can't happen the same way on Windows

On Windows, `CLAUDE_CONFIG_DIR` genuinely relocates `.credentials.json`
(confirmed above, official docs). An agent with no identity binding and a
config dir containing no credentials file would have no readable credential
at all — the CLI has nowhere else to silently fall back to. It would either
prompt to log in or fail outright, consistent with the user's "would be
impossible on Windows" framing, and presumably also what the layer-3 spawn
gate in `inject.rs` was designed assuming would happen everywhere. macOS
breaks that assumption at the OS-integration layer, below anything
AgentMux's own gate can see or control.

## 5. Open questions / not fully explained

- The multiple hash-suffixed Keychain service-name variants observed on this
  machine (`Claude Code-credentials-<hash>`, e.g. `-0b9a1af6`, `-4d6f0fa0`,
  ...) aren't accounted for by either the official docs or issue #9403, which
  both describe a single plain `"Claude Code-credentials"` entry. Possible
  explanations not yet confirmed: leftover entries from multiple installed
  CLI versions, a third-party multi-account switcher tool that scopes
  entries itself, or per-app-instance Keychain access-group hashing by
  macOS. Worth a follow-up if it matters for the fix design in the plan doc.
- **Resolved during follow-up investigation:** yes, the newer
  `db_agent_identity_links`-bound path is equally exposed, and for a more
  fundamental reason than "spot-check whether it behaves differently." The
  root cause (§3) is that macOS Keychain storage in the CLI never consults
  `CLAUDE_CONFIG_DIR` at all, regardless of which AgentMux subsystem set
  that env var or why. Whether the value came from the srv-side identity
  gate's `SecretRef::OAuthConfigDir` injection or the CEF-side
  `ensureAuthDir` shared-provider default, the CLI ignores it identically on
  macOS. The identity-account path is not a separate case to verify — it's
  covered by the same mechanism. (Separately, confirmed the srv-side layer-3
  gate's `use_ambient_login` bypass no longer exists as live code — see §2 —
  so it's not a factor for either path.)

## 6. Follow-up

Remediation options are written up separately:
[PLAN_MACOS_CLAUDE_KEYCHAIN_CREDENTIAL_ISOLATION_2026_08_17.md](../specs/PLAN_MACOS_CLAUDE_KEYCHAIN_CREDENTIAL_ISOLATION_2026_08_17.md).
