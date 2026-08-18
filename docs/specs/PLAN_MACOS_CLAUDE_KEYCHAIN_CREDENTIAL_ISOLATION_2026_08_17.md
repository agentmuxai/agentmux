# Plan — enforce the same per-agent Claude auth isolation on macOS that already holds on Windows

**Date:** 2026-08-17
**Status:** proposed, not yet implemented. First slice of 3d shipped
2026-08-17 (see §7) — the rest is intentionally on hold, see §7.
**Context:** follow-up to
[retro-macos-keychain-credential-isolation-gap-2026-08-17.md](../retro/retro-macos-keychain-credential-isolation-gap-2026-08-17.md),
which found that AgentMux's `CLAUDE_CONFIG_DIR`-based per-agent credential
isolation is a silent no-op on macOS, because the Claude Code CLI's macOS
credential storage (OS Keychain) ignores `CLAUDE_CONFIG_DIR` entirely and
always resolves to one machine-wide, OS-user-scoped Keychain entry — a
behavior that only exists on macOS; Linux and Windows both genuinely
relocate the credential file under `CLAUDE_CONFIG_DIR`.

## 1. Problem restated

AgentMux's identity-isolation model (both the current
`db_agent_identity_links` system and the older shared-provider default)
assumes that setting `CLAUDE_CONFIG_DIR` per spawned process controls which
credential that process can see. That assumption is false on macOS.
Concretely, on macOS today:

- An agent with **no** identity-account binding and **no**
  `use_ambient_login` flag can still run fully authenticated, because it
  transparently inherits whatever OAuth session is sitting in the
  machine-wide Keychain entry — not because of any AgentMux-side bypass, but
  because the underlying CLI never consulted AgentMux's config dir in the
  first place.
- Two AgentMux-spawned agents given *different* `CLAUDE_CONFIG_DIR` values,
  intended to be different accounts, will silently authenticate as the
  **same** account on macOS if a Keychain login already exists — there's no
  way today to detect this from inside AgentMux, since every layer only
  checks its own `db_agent_identity_links` bookkeeping, not what credential
  the CLI actually resolved.
- Any other process on the machine running as the same OS user can read that
  Keychain entry outright (documented ACL weakness, retro §3), which is a
  second, compounding isolation gap beyond AgentMux's own control.

The goal: make macOS behave like Windows/Linux — an agent with no valid
binding should have no usable credential, and different identity-bound
agents should not be able to silently share one Keychain login.

## 2. Constraints

- The macOS Keychain behavior is upstream Claude Code CLI behavior, not
  something AgentMux controls directly. Any fix has to work *around* it
  (force a different credential-supply mechanism) rather than *through* it —
  there's no documented `CLAUDE_CONFIG_DIR`-equivalent flag for macOS
  Keychain scoping.
- Whatever mechanism is chosen must not break the documented, sanctioned
  `use_ambient_login = 1` path, which is *supposed* to fall through to
  whatever's ambiently available.
- Must not regress the existing Linux/Windows isolation, which already works
  correctly via `CLAUDE_CONFIG_DIR`.

## 3. Candidate fixes, ranked

**Recommended: 3a as the primary fix, landed together with 3d's
detection/fail-closed check as a safety net. Hold 3b/3c as documented but
not pursued unless 3a proves insufficient.**

**3a. Force file-based credential storage on macOS the same way Linux does,
by supplying each agent's credential through `apiKeyHelper` instead of
letting the CLI touch Keychain at all.**
The CLI's authentication precedence (confirmed via the official docs,
"Authentication precedence" section) checks several sources before falling
through to the stored `/login` OAuth credential — notably `apiKeyHelper`
(precedence source 4) ranks *above* the OAuth/Keychain credential (source
7). AgentMux could point `apiKeyHelper` at a small per-agent script that
reads a token from the agent's own isolated `CLAUDE_CONFIG_DIR`-scoped file,
achieving real filesystem-level isolation identical in spirit to the Linux
path, without touching Keychain at all. This requires AgentMux to
independently manage OAuth token refresh (since it's bypassing the CLI's own
Keychain-backed refresh flow) — the main implementation cost.
*Tradeoff:* most work, but the only option that gives macOS the same
filesystem-scoped guarantee Linux/Windows already have, and doesn't depend
on any undocumented or fragile Keychain behavior.

**3b. Use `CLAUDE_CODE_OAUTH_TOKEN` per spawned process, sourced from an
AgentMux-managed per-identity token store.**
This env var is documented (precedence source 5) to bypass Keychain reads.
Simpler than 3a since it doesn't require an `apiKeyHelper` script.
**Rejected as primary fix** — GitHub issue #37512 documents that setting
this variable causes the CLI to silently *delete* the machine's shared
Keychain entry on process exit, which would destroy the ambient login other
non-AgentMux-managed `use_ambient_login=1` sessions (or a developer's own
interactive `claude` session on the same Mac) depend on. Usable only if
AgentMux can guarantee no other process on the machine still needs the plain
Keychain entry — too fragile an assumption to rely on by default.

**3c. Request Keychain ACL hardening / a `CLAUDE_CONFIG_DIR`-scoped Keychain
service name from Anthropic upstream.**
The root cause is entirely inside the CLI's macOS storage code. The cleanest
long-term fix is upstream: either honor `CLAUDE_CONFIG_DIR` for Keychain
service-name scoping on macOS the same way it's honored for the file path on
Linux/Windows, or at minimum tighten the ACL per the Silverfort disclosure.
Worth filing/tracking, but not something AgentMux can land on its own
timeline — listed for completeness, not as something this plan can execute
directly.

**3d. Fail-closed detection: after spawn, verify the *actual* resolved
credential matches the intended identity, not just that the gate ran.**
Independent of which of 3a-3c is chosen, add a runtime check (e.g., shell
out to the CLI's own credential-source introspection immediately after
spawn, or before allowing the first request) that confirms the
account/session AgentMux *expects* to be active is the one actually in use —
rather than trusting that setting `CLAUDE_CONFIG_DIR` was sufficient. This
directly closes the blind spot found in this investigation: today nothing in
AgentMux ever checks what credential a spawned process actually ended up
using. Cheap relative to 3a/3b, and valuable as a safety net even after 3a
lands, since it would catch any future regression of this class immediately
instead of requiring another manual Keychain-dump investigation.

## 4. Non-goals

- Not attempting to patch or vendor a fork of the Claude Code CLI to change
  its Keychain behavior directly — out of scope and a maintenance burden;
  prefer working through documented credential-precedence hooks (3a) or
  upstream engagement (3c).
- Not proposing 3b as shipped default behavior, given the confirmed
  data-loss bug (#37512) — only worth revisiting if that upstream bug is
  fixed.
- Not scoping this plan to Linux, which is unaffected (already file-based
  and correctly relocated by `CLAUDE_CONFIG_DIR`).

## 5. Open questions (carried from the retro)

- **Resolved:** both the `db_agent_identity_links`-bound path and the legacy
  shared-provider default fall through to ambient Keychain on macOS
  identically — the root cause is in the CLI's OS-level credential storage,
  which doesn't distinguish which AgentMux subsystem set `CLAUDE_CONFIG_DIR`
  (see retro §5). This *simplifies* 3a's scope: every call site that
  currently relies on `CLAUDE_CONFIG_DIR` for isolation needs the
  `apiKeyHelper` treatment on macOS, with no special-casing between the two
  systems. Full call-site list: `agentmux-srv/src/backend/providers.rs:60`,
  `native_memory_handlers.rs:46,69`, `cli_handlers.rs:305`,
  `app_api/agent_open.rs:309`, `subagent_watcher/parse.rs:487`,
  `agentmux-cef/src/commands/providers.rs:393-491`, plus the CEF-side
  `ensure_auth_dir` (`agentmux-cef/src/commands/platform.rs:79`) and its 5
  frontend callers (`agent-model.ts`, `PreLaunchAuthPanel.tsx`,
  `useAgentControllerStatus.ts`, `ClaudeLoginPanel.tsx`).
- The unexplained hash-suffixed Keychain entries (retro §5) — worth
  understanding before 3a ships, in case they indicate an existing,
  undocumented partial mitigation already in place (e.g., from a prior CLI
  version) that 3a would need to account for or clean up.
- Whether 3a's `apiKeyHelper`-managed token refresh can reuse the CLI's own
  refresh-token exchange logic or needs an independent reimplementation —
  not yet investigated, and likely the single biggest cost driver for 3a.

## 6. Suggested implementation order

1. Spot-check the open question in §5 (identity-account path vs. legacy
   shared path) — cheap, and changes 3a's scope.
2. Prototype 3d (fail-closed detection) first — it's the cheapest of the
   four options, ships independent value immediately (turns a silent
   isolation gap into a loud, actionable failure), and gives a concrete test
   harness to validate 3a against once it's built.
3. Build 3a (`apiKeyHelper`-backed per-agent file credential) behind the
   detection check from step 2, so a regression during development is
   caught by the same mechanism rather than requiring another manual
   Keychain investigation.
4. File the upstream ask (3c) in parallel — long lead time, doesn't block
   1-3.

## 7. Progress (2026-08-17)

**Step 1 (§5's open question) done** — resolved by re-reading
`inject_identity_env_with_broker` in full: both credential paths are
affected identically, and are affected regardless of the srv-side identity
gate's own policy history (its `use_ambient_login` bypass, referenced in
the original retro draft, turned out to already be fully retired — see the
retro's §2 correction). See retro §5.

**A bounded first slice of 3d shipped**, narrower than the full "verify the
actual resolved credential matches the intended identity" detector
originally scoped: while investigating, found that the *existing*
`probe_oauth_status()` (`agentmux-srv/src/identity/resolver/oauth_probe.rs`)
already treats a missing `<dir>/.credentials.json` as a definitive
`needs_reauth` signal — which, given §3's root cause, means every
Keychain-backed macOS Claude account was almost certainly being mislabeled
`needs_reauth` even while genuinely working. Verified empirically against a
real per-identity bundle dir on the dev machine (zero `.credentials.json`
present). Fixed: on macOS, a missing token file for `claude` now returns
`None` (status left alone) instead of asserting a false `needs_reauth` —
see the function's updated doc comment for the full reasoning, and its test
module for the platform-gated coverage (`#[cfg(target_os = "macos")]` /
`#[cfg(not(target_os = "macos"))]`). Two pre-existing integration tests in
`inject.rs` that exercised this exact scenario under provider `"claude"`
were repointed to `"codex"` (unaffected by the carve-out) so they keep
testing the general probe→upsert plumbing rather than colliding with the
new macOS-specific behavior.

This is real, shipped value (removes a standing false-positive), but it is
**not** the full 3d design (an active post-spawn check that fails loudly on
a genuine cross-account mix-up) — it's a correctness fix to a signal 3d
would have needed anyway. The full active-detection design is still open.

**Decision (2026-08-17): stop here for now.** 3a (`apiKeyHelper`-backed
credential storage, requiring AgentMux to independently manage OAuth token
refresh against Anthropic's undocumented endpoint) is explicitly *not*
being started — the risk/cost this plan already flagged in §5's last bullet
held up under scrutiny, and it's being left as a scoped follow-up rather
than started blind. Filing the upstream ask (3c) is also being held pending
a separate go-ahead, since it's a public action (a GitHub issue on
`anthropics/claude-code`) that shouldn't be taken without explicit sign-off.
