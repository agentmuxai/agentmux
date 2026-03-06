# CLI Installation Research (March 2026)

## Summary Table

| Provider | npm Package | Latest Version | Node.js Min | Native Install? | Verify Command |
|----------|------------|----------------|-------------|-----------------|----------------|
| Claude | `@anthropic-ai/claude-code` | native binary (npm deprecated) | 18+ (npm only) | Yes — standalone binary | `claude --version` / `claude doctor` |
| Codex | `@openai/codex` | 0.107.0 | 22+ | Yes — GitHub releases | `codex --version` |
| Gemini | `@google/gemini-cli` | 0.31.0 | 20+ | No (npm/brew only) | `gemini --version` |

---

## Claude Code

**Status:** npm install is **deprecated**. Native binary is the official method.

### Install Commands

| Platform | Command |
|----------|---------|
| macOS/Linux/WSL | `curl -fsSL https://claude.ai/install.sh \| bash` |
| Windows PowerShell | `irm https://claude.ai/install.ps1 \| iex` |
| Windows CMD | `curl -fsSL https://claude.ai/install.cmd -o install.cmd && install.cmd && del install.cmd` |
| Homebrew (macOS) | `brew install --cask claude-code` |
| WinGet (Windows) | `winget install Anthropic.ClaudeCode` |
| npm (deprecated) | `npm install -g @anthropic-ai/claude-code` |

### Key Details
- Native binary is self-contained, no Node.js dependency
- Auto-updates in background (disable with `DISABLE_AUTOUPDATER=1`)
- Can pin version: `curl -fsSL https://claude.ai/install.sh | bash -s 1.0.58`
- Can select channel: `stable` (~1 week old) or `latest`
- System requirements: macOS 13+, Windows 10 1809+, Ubuntu 20.04+, 4GB RAM
- Windows requires Git for Windows
- Auth: `claude auth login` → opens browser (requires paid plan: Pro/Max/Teams/Enterprise)
- Auth check: `claude auth status --json` → `{loggedIn, authMethod, email, subscriptionType}`

---

## OpenAI Codex

**Status:** npm is the primary install method. Also has native binaries on GitHub.

### Install Commands

| Platform | Command |
|----------|---------|
| npm (all platforms) | `npm install -g @openai/codex` |
| Homebrew (macOS) | `brew install --cask codex` |
| GitHub Releases | Download from https://github.com/openai/codex/releases |

### Key Details
- Latest version: **0.107.0** (March 2, 2026)
- Requires Node.js **22+** (high requirement)
- Auth: first run prompts for ChatGPT account or OpenAI API key
- Auth check: `codex login status` → exit code 0 = logged in
- Interactive TUI on `codex` launch
- Included with ChatGPT Plus/Pro/Business/Edu/Enterprise plans

---

## Google Gemini CLI

**Status:** npm is the primary install method. Also available via Homebrew on macOS.

### Install Commands

| Platform | Command |
|----------|---------|
| npm (all platforms) | `npm install -g @google/gemini-cli` |
| Homebrew (macOS) | `brew install gemini-cli` |
| MacPorts (macOS) | `sudo port install gemini-cli` |
| npx (no install) | `npx @google/gemini-cli` |

### Key Details
- Latest version: **0.31.0**
- npm package: `@google/gemini-cli` (NOT `@anthropic-ai/gemini-cli` — we had this WRONG)
- Requires Node.js **20+**
- Auth: `gemini auth login` → opens browser (Google OAuth)
- Auth check: `gemini auth status` → exit code 0 = logged in
- Release channels: stable (default), preview, nightly

---

## Implications for AgentMux

### Current Code Issues

1. **Gemini npm package is WRONG** — code has `@anthropic-ai/gemini-cli`, should be `@google/gemini-cli`
2. **Claude should NOT use npm** — should use native installer (`curl` / PowerShell script)
3. **Node.js 22 for Codex** — users may not have this, need to check or bundle
4. **npm spawning visible console** — on Windows, `npm.cmd` opens a console window

### Recommended Install Strategy

| Provider | Strategy |
|----------|----------|
| Claude | Run `curl -fsSL https://claude.ai/install.sh \| bash` (Unix) or `irm https://claude.ai/install.ps1 \| iex` (Windows) |
| Codex | `npm install --prefix ~/.agentmux/cli/codex @openai/codex@0.107.0` (pin version) |
| Gemini | `npm install --prefix ~/.agentmux/cli/gemini @google/gemini-cli@0.31.0` (pin version) |

### Detection Priority

Before installing, check in this order:
1. Local AgentMux install dir (`~/.agentmux/cli/<provider>/`)
2. System PATH (`where`/`which` the binary name)
3. If neither found → install

### Verification After Install

```
claude --version    # or claude doctor
codex --version
gemini --version
```

If exit code != 0 or output empty → install failed, show error to user.

---

## Browser Auth Flow Comparison

| | Claude Code | Codex CLI | Gemini CLI |
|---|---|---|---|
| **Browser auto-open** | Yes | Yes | Yes |
| **Separate login cmd** | `claude auth login` | `codex login` | First run / env vars |
| **URL printed to stdout** | Yes (always) | Yes (always) | Only with `--debug` flag |
| **Device code flow** | No | Yes (`--device-auth`) | Yes |
| **API key alternative** | Console API key | `--with-api-key` flag | `GEMINI_API_KEY` env var |
| **Callback port** | Varies | `localhost:1455` | Google redirect (varies) |

### PTY Capture Notes

- **Claude & Codex** — OAuth URL always appears in PTY stdout output. Existing AgentMux regex capture pattern works.
- **Gemini** — needs `--debug` flag to print the URL to stdout. Without it, browser opens silently via OS. May need to add `--debug` to auth args, or rely on OS browser launch from the sidecar process context.
