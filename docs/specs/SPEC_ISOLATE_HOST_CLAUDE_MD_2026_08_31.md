# Spec: Stop the isolated Claude Code config dir from falling back to the host's `~/.claude/CLAUDE.md`

**Date:** 2026-08-31
**Status:** Proposed
**Motivated by:** direct request to verify that AgentMux agents don't use
the host's global Claude Code config — verification found a real, live
leak, not just a documentation gap.

## Problem

The Armory "Global Memory" pane (`GlobalBrainManager`) shows a read-only
reference panel labeled **"Claude Code — host CLI config"** with the
tooltip *"Claude Code's own config on this machine. Not read by spawned
in-app agents — see the block above for what they use."* That claim is
**false as implemented.**

### Verified evidence (this machine, live)

- AgentMux correctly sets `CLAUDE_CONFIG_DIR` to an isolated per-
  provider or per-identity directory for every spawned agent
  (`agent_open.rs`'s `provider_auth_dir()` for default agents,
  `identity/resolver/inject.rs`'s bound-account injection for identity-
  bound ones). This part of the isolation design is real and correctly
  wired — confirmed by reading both code paths directly, not just specs.
- **None of the 18+ isolated `.../claude` config directories on this
  machine — spanning every channel, identity, and the shared default
  provider dir — have ever had a `CLAUDE.md` file written into them.**
- A live agent session (this one) with `CLAUDE_CONFIG_DIR` correctly
  pointed at an isolated identity directory nonetheless received the
  full contents of the real `C:\Users\<user>\.claude\CLAUDE.md` in its
  system prompt.

### Root cause

`CLAUDE_CONFIG_DIR` relocates Claude Code CLI's credential/session/
project storage (confirmed on-disk: `.credentials.json`, `.claude.json`,
`projects/`, `sessions/`), but AgentMux never writes a `CLAUDE.md` into
that relocated directory. Claude Code CLI's own user-level `CLAUDE.md`
discovery does not treat an isolated dir with no `CLAUDE.md` as "no
user-level memory file" — it falls through to the real
`$HOME/.claude/CLAUDE.md` instead. The net effect: every AgentMux-
spawned Claude Code agent on this machine has likely been silently
inheriting the operator's personal global instructions file, the exact
thing the isolation boundary exists to prevent.

This is a **different** bug from the one already fixed in
`SPEC_BLOCK_AMBIENT_HOME_DIR_IDENTITY_BINDING_2026_08_25.md` (which
blocks spawn when an identity's configured dir literally *is* the
ambient home). That fix doesn't help here: the leak reproduces even when
`CLAUDE_CONFIG_DIR` points at a genuinely separate, correctly-isolated
directory — the problem is an *empty* isolated directory, not a
misdirected one.

## Design

Seed an empty placeholder `CLAUDE.md` into a `claude`-provider isolated
config dir the first time it's used, so Claude Code CLI always finds
*something* there and never falls through to the real home. Idempotent
and non-destructive: only ever written when no `CLAUDE.md` already
exists at that path, so it can never clobber content Global Memory (or
anything else) has legitimately placed there.

### `agentmux-srv/src/backend/providers.rs`

New function, next to the existing `is_provider_ambient_home_dir` (same
file already owns the provider/dir isolation logic both call sites use):

```rust
pub fn seed_claude_md_placeholder_if_missing(
    provider: &ProviderConfig,
    config_dir: &str,
) -> std::io::Result<bool> {
    if provider.auth_dir_name != "claude" {
        return Ok(false); // Claude-CLI-specific fallback behavior; unverified for other providers.
    }
    let path = std::path::Path::new(config_dir).join("CLAUDE.md");
    if path.exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(config_dir)?;
    std::fs::write(&path, CLAUDE_MD_ISOLATION_PLACEHOLDER)?;
    Ok(true)
}
```

Placeholder content explains itself (a curious operator who opens the
file shouldn't be confused by an unexplained empty file):

```
<!--
AgentMux: intentionally empty.

This is an isolated Claude Code config directory (CLAUDE_CONFIG_DIR),
separate from your personal ~/.claude/CLAUDE.md, so this agent never
silently inherits your personal global instructions. To give every
agent shared instructions, use Armory -> Memory -> Global instead —
those compose into this agent's own project-level CLAUDE.md at launch,
not this file. See SPEC_ISOLATE_HOST_CLAUDE_MD_2026_08_31.md.
-->
```

### Call site 1 — default agents: `agentmux-srv/src/server/app_api/agent_open.rs`

Right after the existing `create_dir_all(&auth_dir)` / `CLAUDE_CONFIG_DIR`
env-var insert (~line 311-312), which already runs unconditionally on
every `agent.open`:

```rust
let _ = std::fs::create_dir_all(&auth_dir);
env_vars.insert(provider.auth_config_dir_env_var.to_string(), json!(auth_dir));
if let Err(e) = providers::seed_claude_md_placeholder_if_missing(provider, &auth_dir) {
    tracing::warn!(auth_dir = %auth_dir, error = %e, "failed to seed CLAUDE.md isolation placeholder");
}
```

### Call site 2 — identity-bound agents: `agentmux-srv/src/identity/resolver/inject.rs`

Inside the existing `if let Some(provider_cfg) = ...` block (~line
574-593) that already looks up `provider_cfg` to run the ambient-home-
dir check — added right after that check passes (i.e., only once we
know `dir` is NOT the ambient home, which is exactly the case this spec
targets):

```rust
if let Some(provider_cfg) =
    crate::backend::providers::get_provider(&resolve_provider_alias(&binding.provider))
{
    if crate::backend::providers::is_provider_ambient_home_dir(provider_cfg, &dir) {
        // ... existing block-spawn logic, unchanged ...
    }
    if let Err(e) = crate::backend::providers::seed_claude_md_placeholder_if_missing(provider_cfg, &dir) {
        tracing::warn!(target: "identity", dir = %dir, error = %e, "failed to seed CLAUDE.md isolation placeholder");
    }
}
env_vars.insert(config_dir_env_var.to_string(), dir.clone());
```

This path runs on every message send (per `inject_identity_env_with_broker`),
so the seed check is a cheap `path.exists()` after the first run.

### Tests

`agentmux-srv/src/backend/providers.rs`, unit tests next to
`is_provider_ambient_home_dir`'s own tests, using a tempdir (same
pattern `cli_handlers.rs`'s `selfheal_tests` already establishes):

- writes the placeholder when `CLAUDE.md` is missing, for the `claude`
  provider.
- does not overwrite an existing `CLAUDE.md` (any content — simulates
  Global Memory or a user having legitimately placed one there).
- no-ops for a non-`claude` provider (e.g. `codex`) even when its
  config dir has no `CLAUDE.md` — this is a Claude-CLI-specific
  fallback behavior, not assumed for any other provider without
  separate verification.
- creates the config dir first if it doesn't exist yet (mirrors the
  `create_dir_all` already done by both call sites, so the function is
  safe to call standalone too).

## Non-goals

- **No change to identity-bound accounts whose configured dir already
  resolves to the ambient home** — that case is already blocked
  entirely by `SPEC_BLOCK_AMBIENT_HOME_DIR_IDENTITY_BINDING_2026_08_25.md`;
  this spec's seed function never runs for that case (spawn is refused
  before reaching it).
- **No attempt to import or compose the operator's real `~/.claude/CLAUDE.md`
  content into the isolated dir.** An empty placeholder is deliberate —
  Global Memory (composed into the agent's project-level `CLAUDE.md`
  instead) is the existing, correct mechanism for shared instructions;
  silently blending the operator's personal global rules into every
  agent would reintroduce exactly the leak this spec closes, just
  copied instead of live-read.
- **No change for non-`claude` providers** (Codex, Gemini, etc.) — this
  spec only verified the leak for Claude Code CLI's specific
  `CLAUDE_CONFIG_DIR` + `CLAUDE.md` discovery behavior. Whether other
  providers have an analogous gap for their own config-dir env var is
  unverified and out of scope here.
- **No change to the GlobalBrainManager UI copy** describing "host CLI
  config" as reference-only — that framing is correct in intent (it's
  meant to be read-only reference, never composed into agent
  instructions); this spec fixes the code so the claim is actually true,
  rather than rewording the claim.
