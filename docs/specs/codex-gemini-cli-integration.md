# Spec: Codex CLI & Gemini CLI — Install, Auth, and Launch Integration

**Date:** 2026-03-19
**Status:** Draft
**Priority:** High — Codex and Gemini agents currently fail on first launch

---

## Problem Statement

When a user selects a Codex or Gemini agent from the Forge, the launch fails with:
```
[agent] AgentY selected (provider: Codex CLI)
[env] working directory: ~/.agentmux/agents/agenty
[cli] checking for codex...
[cli] install failed: The filename, directory name, or volume label syntax is incorrect.
[error] codex not available — install manually or check your internet connection
```

The Claude agent works end-to-end (install → auth → launch) because it has:
1. A dedicated PowerShell/bash installer (`irm https://claude.ai/install.ps1 | iex`)
2. A fast auth check via credentials file (`~/.claude/.credentials.json`)
3. A well-tested stream-json output parser

Codex and Gemini are defined in the provider registry but their install/auth/launch paths have gaps.

---

## Current State

### What Works
- Provider definitions exist in `frontend/app/view/agent/providers/index.ts`
- `ResolveCliCommand` in the backend has 3-step resolution (versioned → system PATH → install)
- `CheckCliAuthCommand` supports all 3 providers
- Forge seed includes Codex (AgentY) and Gemini (AgentZ) agents
- Agent config writing (CLAUDE.md, .mcp.json, skills) works for all providers

### What's Broken

#### 1. Installation Path (Windows)

**Codex** `windowsInstallCommand`: `"npm install -g @openai/codex"`
**Gemini** `windowsInstallCommand`: `"npm install -g @google/gemini-cli"`

The "filename, directory name, or volume label syntax is incorrect" error comes from the backend's `ResolveCliCommand` trying to run the install. Likely causes:
- The install command is run in a context where `npm` isn't on PATH
- The working directory for the install subprocess is invalid
- Windows path separators in the versioned install directory (`~/.agentmux/instances/v0.32.43/cli/codex/`)

**Claude** uses a dedicated installer (`irm https://claude.ai/install.ps1 | iex`) that handles PATH setup internally. Codex and Gemini rely on `npm` being available globally.

#### 2. npm Local Install Path

The backend `ResolveCliCommand` tries `npm install --prefix <dir> <package>@<version>` for local installation. On Windows, the resulting binary is at:
```
~/.agentmux/instances/v<version>/cli/<provider>/node_modules/.bin/<provider>.cmd
```

Issues:
- The `.cmd` extension must be appended on Windows
- The `--prefix` path may have issues with Windows long paths or spaces
- npm may not be available in the subprocess environment

#### 3. Authentication

**Codex auth:**
- Check: `codex login status` (exit code 0 = authenticated)
- Login: `codex login` (opens browser for OpenAI OAuth)
- No fast path (no local credentials file to check)
- API key alternative: `OPENAI_API_KEY` environment variable

**Gemini auth:**
- Check: `gemini auth status` (exit code 0 = authenticated)
- Login: `gemini auth login` (opens browser for Google OAuth)
- No fast path
- API key alternative: `GOOGLE_API_KEY` or `GEMINI_API_KEY` environment variable

**Current gap:** The agent model's auth flow (`agent-model.ts`) is optimized for Claude's OAuth flow. It captures OAuth URLs from PTY output and opens them externally. Codex and Gemini may have different auth URL patterns.

#### 4. Output Parsing

- `styledOutputFormat: "codex-json"` and `"gemini-json"` are defined but the stream parsers may not be implemented
- The `ParserCallbacks` system in `agent-model.ts` is designed for Claude's `stream-json` format
- Codex uses `--full-auto` mode — output format needs investigation
- Gemini uses `--yolo` mode — output format needs investigation

---

## Proposed Fix Plan

### Phase 1: Fix Installation (Critical)

**1a. Fix local npm install on Windows**

In `cli_installer.rs` / `ResolveCliCommand`:
- Ensure `npm` is resolved via full path (check `where npm` first)
- Handle Windows path normalization (forward slashes → backslashes for subprocess)
- Append `.cmd` extension when checking for installed binary on Windows
- Add proper error messages with actionable guidance

**1b. Add fallback to global install**

If local `npm install --prefix` fails:
1. Try `npm install -g <package>@<version>`
2. Try the provider's `windowsInstallCommand` / `unixInstallCommand`
3. Check system PATH again after install
4. If all fail: show clear error with manual install instructions

**1c. Validate install works**

After installation, run `<cli> --version` to confirm the binary works before proceeding.

### Phase 2: Fix Authentication

**2a. Support API key auth for Codex**

Codex can authenticate via `OPENAI_API_KEY` environment variable. The Forge agent config should support setting this:
- Add `api_key` field to ForgeAgent or ForgeContent
- If set, pass as environment variable to the subprocess
- Skip OAuth flow when API key is present
- UI: add API key input field in Forge agent settings

**2b. Support API key auth for Gemini**

Same pattern as Codex but with `GOOGLE_API_KEY` or `GEMINI_API_KEY`.

**2c. OAuth flow for Codex/Gemini**

Codex and Gemini both support interactive OAuth login:
- `codex login` — opens browser for OpenAI login
- `gemini auth login` — opens browser for Google login

The agent model's `handleTerminalData()` already captures OAuth URLs from PTY output. Verify the URL regex patterns work for:
- OpenAI: `https://platform.openai.com/...` or `https://auth0.openai.com/...`
- Google: `https://accounts.google.com/...`

If the regex doesn't match, add provider-specific URL patterns to the auth state machine.

### Phase 3: Fix Output Parsing

**3a. Codex output format investigation**

Codex CLI with `--full-auto`:
- Does it produce structured JSON output?
- What format? (NDJSON, SSE, custom?)
- Can we use `--output-format json` or similar flag?
- If no structured output: use raw mode (display PTY output as-is)

**3b. Gemini output format investigation**

Gemini CLI with `--yolo`:
- Same questions as Codex
- Does `gemini --output-format json` exist?

**3c. Implement parsers if needed**

If structured output is available:
- Add `codex-json` parser to the stream parser system
- Add `gemini-json` parser
- Map events to the existing `ParserCallbacks` interface

If no structured output:
- Use `outputFormat: "raw"` (already configured as default)
- Display raw terminal output in the agent pane
- This is functional but loses the structured tool-call display

### Phase 4: End-to-End Testing

For each provider (Claude, Codex, Gemini):
1. Fresh install test — no CLI pre-installed, launch agent, verify auto-install
2. Auth test — verify OAuth or API key flow works
3. Launch test — send a prompt, verify output displays
4. Restart test — close and reopen agent, verify it reconnects
5. Cross-platform — test on Windows, macOS, Linux

---

## Implementation Details

### Backend Changes (`agentmuxsrv-rs`)

**`cli_installer.rs` / `websocket.rs`:**

```rust
// Fix: resolve npm path before running install
fn resolve_npm_path() -> Option<PathBuf> {
    // Check common locations:
    // - Windows: C:\Program Files\nodejs\npm.cmd
    // - macOS/Linux: /usr/local/bin/npm, /usr/bin/npm
    // - nvm: ~/.nvm/versions/node/*/bin/npm
    // - fnm: similar
    which::which("npm").ok()
}

// Fix: Windows binary path resolution
fn get_cli_binary_path(provider: &str, install_dir: &Path) -> PathBuf {
    let bin_dir = install_dir.join("node_modules/.bin");
    if cfg!(windows) {
        bin_dir.join(format!("{}.cmd", provider))
    } else {
        bin_dir.join(provider)
    }
}
```

### Frontend Changes

**`providers/index.ts`:**

```typescript
// Add API key environment variable names
export interface ProviderDefinition {
    // ... existing fields ...
    apiKeyEnvVar?: string;  // e.g. "OPENAI_API_KEY" for Codex
}

// Update providers:
codex: {
    // ...
    apiKeyEnvVar: "OPENAI_API_KEY",
},
gemini: {
    // ...
    apiKeyEnvVar: "GOOGLE_API_KEY",
},
```

**`agent-model.ts`:**

```typescript
// Add provider-specific OAuth URL patterns
const OAUTH_URL_PATTERNS: Record<string, RegExp> = {
    claude: /https:\/\/claude\.ai\/oauth\/authorize\S+/,
    codex: /https:\/\/(platform\.openai\.com|auth0\.openai\.com)\S+/,
    gemini: /https:\/\/accounts\.google\.com\S+/,
};
```

### Forge UI Changes

**Agent settings panel:**
- Add "API Key" field (password input) for Codex and Gemini agents
- If API key is set, show "Authenticated via API key" status
- If not set, show "Run `<cli> auth login` to authenticate" guidance
- Store API key in ForgeContent (encrypted or in secure store)

---

## Files to Modify

| File | Changes |
|------|---------|
| `agentmuxsrv-rs/src/commands/cli_installer.rs` | Fix npm path resolution, Windows binary paths |
| `agentmuxsrv-rs/src/server/websocket.rs` | Fix `ResolveCliCommand` install fallback chain |
| `frontend/app/view/agent/providers/index.ts` | Add `apiKeyEnvVar` field |
| `frontend/app/view/agent/agent-model.ts` | Add provider-specific OAuth URL patterns |
| `frontend/app/view/forge/forge-view.tsx` | Add API key input in agent settings |

---

## Priority Order

1. **Fix npm install path on Windows** — unblocks Codex and Gemini immediately
2. **API key auth support** — simplest auth path, no OAuth flow needed
3. **OAuth URL patterns** — for users who prefer browser-based auth
4. **Output parsers** — nice-to-have, raw mode works as fallback

---

## Open Questions

1. Does Codex CLI have a structured JSON output mode? (Need to test with `codex --help`)
2. Does Gemini CLI have a structured JSON output mode?
3. Should API keys be stored in the Forge database or in a system keychain?
4. Do we need provider-specific stream parsers, or is raw terminal output acceptable for v1?
5. Codex `--full-auto` and Gemini `--yolo` flags — are these the right defaults for AgentMux's use case?
