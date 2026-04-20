# Kimi Code CLI Provider Integration Spec

**Version:** 0.1  
**Date:** 2026-04-20  
**Status:** Draft — ready for implementation  
**Author:** AgentMux Analysis  

---

## 1. Executive Summary

Integrate [Kimi Code CLI](https://github.com/MoonshotAI/kimi-cli) (`kimi`) as a first-class agent provider in AgentMux, alongside Claude Code, Codex CLI, and Gemini CLI.

**Why Kimi?** Kimi is a mature CLI agent (v1.37.0) with a well-documented JSON streaming format, ACP server mode, and Wire protocol. It supports the Moonshot Kimi model family (k2, etc.) and is already installed in this environment via `uv`.

**Strategic Goal:** Give AgentMux users a non-Anthropic/OpenAI/Google option that supports Chinese and English contexts, with competitive reasoning capabilities.

---

## 2. Kimi Protocol Analysis

Kimi exposes **three** programmatic interfaces. We evaluated all three:

### 2.1 Print Mode + `--output-format stream-json` ⭐ RECOMMENDED (Phase 1)

```bash
kimi --print --output-format stream-json --yolo -p "prompt here"
```

- **Output:** NDJSON lines, one message per line.
- **Schema:** Simple message format (`{"role":"assistant","content":[...],"tool_calls":[...]}`)
- **Input:** Plain text via `-p` or stdin.
- **Auto-approve:** `--yolo` auto-approves all tool calls (like Claude's `--dangerously-skip-permissions`).
- **Exit:** Exits automatically after the turn completes.
- **Session resume:** `--continue` or `--session <id>` flags exist but are untested in print mode.

**Sample output (tool call):**
```json
{"role":"assistant","content":[{"type":"think","think":"..."}],"tool_calls":[{"type":"function","id":"tc_1","function":{"name":"Shell","arguments":"{\"command\":\"ls\"}"}}]}
{"role":"tool","tool_call_id":"tc_1","content":[{"type":"text","text":"file1.py\nfile2.py"}]}
{"role":"assistant","content":[{"type":"text","text":"Here are the files..."}]}
```

**Verdict:** Easiest integration. Mirrors Claude/Gemini subprocess model exactly.

### 2.2 Wire Mode (`kimi --wire`) — FUTURE (Phase 2)

```bash
kimi --wire
```

- **Protocol:** JSON-RPC 2.0 over stdio (Wire protocol v1.9).
- **Methods:** `initialize`, `prompt`, `replay`, `steer`, `set_plan_mode`, `cancel`.
- **Bidirectional:** Agent sends `event` notifications and `request` messages (approvals, questions, external tool calls).
- **Powerful:** Full control over approvals, plan mode, steering.

**Verdict:** Powerful but requires a new controller or significant ACP controller adaptation. Kimi's Wire methods (`prompt`, `event`) differ from AgentMux's ACP (`session/prompt`, `session/update`). Save for Phase 2.

### 2.3 ACP Mode (`kimi acp`) — NOT COMPATIBLE

```bash
kimi acp
```

- Kimi's ACP is an IDE integration server built on top of Wire.
- It speaks JSON-RPC 2.0, but **method names and semantics differ** from AgentMux's ACP controller (`AcpController`).
- AgentMux ACP expects: `initialize` → `initialized` → `session/create` → `session/prompt` → `session/update`.
- Kimi Wire uses: `initialize` → `prompt` → `event`/`request`.

**Verdict:** Do not use AgentMux's `AcpController` for Kimi. Would require a `KimiWireController` or protocol adapter.

---

## 3. Architecture Decision

| Aspect | Decision |
|--------|----------|
| **Controller type** | `Subprocess` (Phase 1) |
| **Launch args** | `["--print", "--output-format", "stream-json", "--yolo", "-p", ""]` |
| **Styled output format** | `kimi-stream-json` (new) |
| **Resume flag** | `"--continue"` (tentative — needs validation) |
| **Session ID field** | `"session_id"` (Kimi uses session IDs internally) |
| **Auth isolation** | `KIMI_SHARE_DIR` env var |
| **Auth type** | `api-key` (Kimi uses API key or OAuth via `kimi login`) |
| **Installation** | Phase 1: PATH fallback. Phase 2: `uv tool install` or `pip install`. |

**Rationale for Subprocess over Persistent:**
- Kimi's `--print` mode is explicitly designed for non-interactive, one-turn execution.
- While `--input-format stream-json` exists for persistent streaming, it is less documented and untested in the context of long-running sessions.
- Subprocess gives us session isolation per turn, which aligns with how Claude Code is currently integrated (Claude also uses `Subprocess` with `--resume`).

---

## 4. Files to Modify

### Backend (Rust)

| File | Change |
|------|--------|
| `agentmux-srv/src/backend/providers.rs` | Add `KIMI` `ProviderConfig` static; register in `REGISTRY` and `ORDER`; add alias |
| `agentmux-srv/src/server/app_api.rs` | Add `"kimi" => "kimi-stream-json"` to `output_format` match |
| `agentmux-srv/src/backend/agent_config.rs` | Optionally write `KIMI.md` alongside `CLAUDE.md` if Kimi reads system prompts from a file |
| `agentmux-srv/src/server/cli_handlers.rs` | Add PATH fallback for non-npm CLIs (see Installation Strategy) |

### Frontend (TypeScript)

| File | Change |
|------|--------|
| `frontend/app/view/agent/providers/index.ts` | Add `kimi` to `PROVIDERS` record; extend `outputFormat` union type |
| `frontend/app/view/agent/providers/kimi-translator.ts` | **New file** — translate Kimi NDJSON → `StreamEvent` |
| `frontend/app/view/agent/providers/translator-factory.ts` | Add `kimi-stream-json` case |
| `frontend/app/view/agent/buildRuntimeArgs.ts` | Add Kimi permission flags (`--yolo` mapping) |
| `frontend/app/view/agent/types.ts` | Extend `PermissionMode` or `ModelChoice` if Kimi-specific values needed |

### Docs

| File | Change |
|------|--------|
| `docs/specs/app-api-status.md` | Update Tier 1 status if applicable |

---

## 5. Backend Implementation Details

### 5.1 `providers.rs` — Add Kimi Provider Config

```rust
static KIMI: ProviderConfig = ProviderConfig {
    id: "kimi",
    display_name: "Kimi Code CLI",
    cli_command: "kimi",
    controller_type: ControllerType::Subprocess,
    launch_args: &[
        "--print",
        "--output-format", "stream-json",
        "--yolo",
        "-p", "",
    ],
    persistent_launch_args: None, // Phase 1
    resume_flag: Some("--continue"), // TBD: validate this works in print mode
    session_id_field: "session_id",
    styled_output_format: "kimi-stream-json",
    auth_config_dir_env_var: "KIMI_SHARE_DIR",
    auth_dir_name: "kimi",
    auth_extra_env: &[
        // Kimi stores config under ~/.kimi by default.
        // KIMI_SHARE_DIR redirects this to our isolated auth dir.
    ],
    unset_env: &[],
    npm_package: "", // Kimi is a Python package, not npm
    pinned_version: "",
    icon: "moon", // or suitable icon
    docs_url: "https://moonshotai.github.io/kimi-cli/",
};
```

Update `REGISTRY`:
```rust
m.insert(KIMI.id, &KIMI);
```

Update `ORDER`:
```rust
static ORDER: &[&str] = &["claude", "codex", "gemini", "kimi", "openclaw", "pi"];
```

Add alias:
```rust
m.insert("kimi-cli", "kimi");
m.insert("kimi_code", "kimi");
```

Update tests to expect 6 providers.

### 5.2 `app_api.rs` — Output Format Derivation

```rust
let output_format = match provider.id {
    "claude" => "claude-stream-json",
    "codex" => "codex-json",
    "gemini" => "gemini-json",
    "kimi" => "kimi-stream-json",
    _ => "claude-stream-json",
};
```

### 5.3 `cli_handlers.rs` — PATH Fallback for Non-npm CLIs

**Problem:** Kimi is a Python package installed via `pip`/`uv`, not `npm`. The current `resolvecli` handler only looks in `node_modules/.bin/` and fails if `npm_package` is empty.

**Solution:** Add a PATH fallback step before returning an error.

In `register_cli_handlers` / `COMMAND_RESOLVE_CLI`:

```rust
// After checking versioned npm install dir...
if std::path::Path::new(&npm_bin).exists() { ... }

// NEW: If npm_package is empty, try system PATH.
if cmd.npm_package.is_empty() {
    let path_cmd = if cfg!(windows) {
        format!("{}.cmd", cmd.cli_command)
    } else {
        cmd.cli_command.to_string()
    };
    // Use `which` / `where` to resolve on PATH
    let which_result = if cfg!(windows) {
        tokio::process::Command::new("where").arg(&path_cmd).output().await
    } else {
        tokio::process::Command::new("which").arg(&path_cmd).output().await
    };
    if let Ok(out) = which_result {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").trim();
            if !path.is_empty() && std::path::Path::new(path).exists() {
                let version = get_cli_version(path).await;
                return Ok(Some(serde_json::to_value(&ResolveCliResult {
                    cli_path: path.to_string(),
                    version,
                    source: "system_path".to_string(),
                }).unwrap()));
            }
        }
    }
    return Err(format!(
        "{} not found. Install it and ensure it's on PATH.",
        cmd.cli_command
    ));
}
```

> **Note:** The existing `app_api.rs` also hardcodes the npm binary path when building metadata. It needs to use the path returned by `resolvecli` rather than assuming `node_modules/.bin/`. Verify that `cmd` in block meta already uses the resolved path.

**Verification:** In `app_api.rs` line 119, `npm_bin` is constructed from `provider_dir`. This needs to change to call `resolvecli` first, or fall back to PATH resolution if the npm path doesn't exist. Alternatively, modify the construction to check PATH when `npm_package` is empty.

### 5.4 Auth Isolation

Kimi uses `KIMI_SHARE_DIR` (default `~/.kimi`) for all config, sessions, logs, and auth.

AgentMux should set:
```bash
KIMI_SHARE_DIR=~/.agentmux/config/auth/kimi
```

This gives Kimi its own isolated config directory per AgentMux installation, matching the pattern used for Claude (`CLAUDE_CONFIG_DIR`), Codex (`CODEX_HOME`), and Gemini (`GEMINI_CLI_HOME`).

---

## 6. Frontend Implementation Details

### 6.1 `providers/index.ts` — Add Kimi Definition

```typescript
export interface ProviderDefinition {
    // ... extend outputFormat union:
    outputFormat: "claude-stream-json" | "gemini-json" | "codex-json" | "kimi-stream-json" | "acp" | "raw";
    styledOutputFormat: "claude-stream-json" | "gemini-json" | "codex-json" | "kimi-stream-json" | "acp";
    // ...
}

export const PROVIDERS: Record<string, ProviderDefinition> = {
    // ... existing providers ...
    kimi: {
        id: "kimi",
        displayName: "Kimi Code CLI",
        cliCommand: "kimi",
        defaultArgs: [],
        styledArgs: ["--print", "--output-format", "stream-json", "--yolo", "-p", ""],
        outputFormat: "raw",
        styledOutputFormat: "kimi-stream-json",
        authType: "api-key",
        authCheckCommand: ["info"], // `kimi info` exits 0 if installed and auth ok
        authLoginCommand: ["login"],
        npmPackage: "", // Python package, not npm
        pinnedVersion: "",
        docsUrl: "https://moonshotai.github.io/kimi-cli/",
        windowsInstallCommand: "pip install kimi-cli",
        unixInstallCommand: "pip install kimi-cli",
        icon: "moon",
        unsetEnv: [],
        authConfigDirEnvVar: "KIMI_SHARE_DIR",
        authDirName: "kimi",
        launchArgs: ["--print", "--output-format", "stream-json", "--yolo", "-p", ""],
        resumeFlag: "--continue",
        sessionIdField: "session_id",
        controllerType: "subprocess",
    },
};
```

### 6.2 `providers/kimi-translator.ts` — New Translator

**Design principle:** Mirror `GeminiTranslator` since Kimi's stream-json is similarly simple (discrete messages, not incremental deltas like Claude).

```typescript
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { StreamEvent, ToolCallEvent, ToolResultEvent } from "../types";
import type { OutputTranslator } from "./translator";

/**
 * Translates Kimi Code CLI `--output-format stream-json` events into StreamEvent format.
 *
 * Kimi emits NDJSON with these message types:
 *   {"role":"assistant","content":[...],"tool_calls":[...]}
 *   {"role":"tool","tool_call_id":"...","content":[...]}
 *
 * Content parts can be:
 *   - {"type":"text","text":"..."}
 *   - {"type":"think","think":"..."}
 *   - {"type":"image_url",...} (future)
 *
 * Tool calls use OpenAI-style function calling:
 *   {"type":"function","id":"tc_1","function":{"name":"Shell","arguments":"{...}"}}
 */
export class KimiTranslator implements OutputTranslator {
    private toolNameById: Map<string, string> = new Map();

    translate(rawEvent: any): StreamEvent[] {
        if (!rawEvent || typeof rawEvent !== "object") return [];

        const role: string = rawEvent.role ?? "";

        switch (role) {
            case "assistant": {
                const events: StreamEvent[] = [];

                // Handle content parts (text, think)
                const content = rawEvent.content;
                if (Array.isArray(content)) {
                    for (const part of content) {
                        if (part.type === "text" && part.text) {
                            events.push({ type: "text", content: part.text });
                        } else if (part.type === "think" && part.think) {
                            events.push({ type: "thinking", content: part.think });
                        }
                    }
                } else if (typeof content === "string" && content) {
                    events.push({ type: "text", content });
                }

                // Handle tool_calls
                const toolCalls = rawEvent.tool_calls;
                if (Array.isArray(toolCalls)) {
                    for (const tc of toolCalls) {
                        if (tc.type === "function") {
                            const toolName = tc.function?.name ?? "unknown";
                            const toolId = tc.id ?? `tool-${Date.now()}`;
                            let params: Record<string, any> = {};
                            try {
                                const args = tc.function?.arguments;
                                if (typeof args === "string") {
                                    params = JSON.parse(args);
                                } else if (typeof args === "object" && args !== null) {
                                    params = args;
                                }
                            } catch {
                                params = {};
                            }
                            this.toolNameById.set(toolId, toolName);
                            events.push({ type: "tool_call", tool: toolName, id: toolId, params });
                        }
                    }
                }

                return events;
            }

            case "tool": {
                const toolId: string = rawEvent.tool_call_id ?? "";
                const toolName = this.toolNameById.get(toolId) ?? "unknown";
                const content = rawEvent.content;
                let resultText = "";
                if (Array.isArray(content)) {
                    resultText = content
                        .filter((p: any) => p.type === "text")
                        .map((p: any) => p.text)
                        .join("");
                } else if (typeof content === "string") {
                    resultText = content;
                }
                return [{
                    type: "tool_result",
                    tool: toolName,
                    id: toolId,
                    status: "success",
                    result: { content: resultText },
                }];
            }

            default:
                return [];
        }
    }

    reset(): void {
        this.toolNameById.clear();
    }
}
```

### 6.3 `translator-factory.ts` — Register Kimi Translator

```typescript
import { KimiTranslator } from "./kimi-translator";

export function createTranslator(outputFormat: string): OutputTranslator {
    switch (outputFormat) {
        case "claude-stream-json": return new ClaudeTranslator();
        case "gemini-json": return new GeminiTranslator();
        case "codex-json": return new CodexTranslator();
        case "kimi-stream-json": return new KimiTranslator(); // NEW
        case "acp": return new AcpTranslator();
        default:
            console.warn(`[translator-factory] Unknown output format "${outputFormat}", falling back to Claude translator`);
            return new ClaudeTranslator();
    }
}
```

### 6.4 `buildRuntimeArgs.ts` — Kimi Permission Flags

Kimi uses `--yolo` for auto-approve. Add to `PERMISSION_FLAGS`:

```typescript
const PERMISSION_FLAGS: Record<PermissionMode, string[]> = {
    bypass: ["--yolo"],
    auto: ["--yolo"], // Kimi doesn't have granular permission modes; map all to --yolo for now
    acceptEdits: ["--yolo"],
    plan: ["--yolo"],
    default: ["--yolo"],
};
```

> **Note:** Kimi does not support Claude-style permission modes (`auto`, `acceptEdits`, `plan`). All modes map to `--yolo` in Phase 1. Phase 2 (Wire mode) could support real approval flow.

Update `PERMISSION_STRIP`:
```typescript
const PERMISSION_STRIP = new Set(["--yolo"]);
```

---

## 7. Kimi Stream-JSON → StreamEvent Mapping

| Kimi NDJSON | AgentMux `StreamEvent` | Notes |
|-------------|------------------------|-------|
| `{"role":"assistant","content":[{"type":"text","text":"hi"}]}` | `TextEvent {type:"text", content:"hi"}` | One text part → one text event |
| `{"role":"assistant","content":[{"type":"think","think":"..."}]}` | `ThinkingEvent {type:"thinking", content:"..."}` | Rendered as collapsible thinking block |
| `{"role":"assistant","tool_calls":[{"type":"function",...}]}` | `ToolCallEvent {type:"tool_call", tool, id, params}` | Parse `function.arguments` JSON string |
| `{"role":"tool","tool_call_id":"tc_1","content":[...]}` | `ToolResultEvent {type:"tool_result", tool, id, status:"success", result}` | Aggregate text parts into result string |
| No explicit session end event | N/A | Process exit signals turn end |

**Content part types Kimi supports (future-proofing):**
- `text` — handled
- `think` — handled
- `image_url` — ignore or render as image link (Phase 2)
- `audio_url` — ignore (Phase 2)
- `video_url` — ignore (Phase 2)

---

## 8. Auth & Configuration

### 8.1 Auth Check

Kimi does not have a dedicated "auth status" command like `claude auth status`. The best proxy is:

```bash
kimi info
```

- If authenticated: prints version info, exits 0.
- If not authenticated: prints error, exits non-zero.

**Alternative:** Check for the existence of `~/.kimi/config.toml` or credentials file.

### 8.2 Auth Login

```bash
kimi login
```

Opens browser OAuth flow or prompts for API key.

### 8.3 Config File Generation

Kimi reads configuration from `~/.kimi/config.toml`. When AgentMux isolates auth via `KIMI_SHARE_DIR`, Kimi will read/write to `~/.agentmux/config/auth/kimi/config.toml`.

AgentMux's `agent_config.rs` currently generates:
- `CLAUDE.md` — system prompt for Claude Code
- `.mcp.json` — MCP server config
- `.claude/commands/*.md` — slash commands

**Question:** Does Kimi read a system prompt file like `CLAUDE.md`?  
**Answer:** Unknown. Kimi has `--agent-file` for custom agent specs and `--skills-dir` for skills. It does not appear to auto-read `CLAUDE.md`. For Phase 1, skip `KIMI.md` generation. Forge `soul`/`agentmd` content can be passed via the `-p` prompt or via `--agent-file` in Phase 2.

**MCP:** Kimi supports MCP via `--mcp-config-file`. AgentMux should generate `.mcp.json` and pass it via `--mcp-config-file` in the launch args if Forge content includes MCP config.

---

## 9. Installation Strategy

### Phase 1: PATH Fallback (Minimal Change)

Assume the user has already installed `kimi` on their system via `pip`, `uv`, or official installer. AgentMux's `resolvecli` handler falls back to `where kimi` / `which kimi` when `npm_package` is empty.

**Pros:** Zero installation complexity. Works immediately if `kimi` is on PATH.  
**Cons:** No version pinning. No isolation from system Kimi install.

### Phase 2: Managed Installation (Future)

Extend `resolvecli` to support Python package managers:

```rust
enum InstallMethod {
    Npm { package: String, version: String },
    Pip { package: String, version: String },
    Uv { package: String, version: String },
}
```

For Kimi:
```bash
# Via uv (recommended, creates isolated environment)
uv tool install kimi-cli==1.37.0

# Via pip
pip install --target ~/.agentmux/<version>/cli/kimi kimi-cli==1.37.0
```

This requires significant changes to `cli_handlers.rs` and the provider schema. Defer to Phase 2.

---

## 10. Session & Resume

Kimi manages sessions in `~/.kimi/sessions/` (or `$KIMI_SHARE_DIR/sessions/`). Session IDs are UUIDs.

**Flags:**
- `--continue` — resume previous session for working directory
- `--session <id>` — resume specific session
- `--print` — non-interactive mode (implicitly adds `--yolo`)

**Open Question:** Does `--print --continue` work? The docs don't explicitly confirm this combination. We need to test:

```bash
kimi --print --continue --output-format stream-json -p "second message"
```

If it works, the `resume_flag: Some("--continue")` config is correct. If not, we may need to:
1. Parse the session ID from the first turn's stderr (`To resume this session: kimi -r <id>`)
2. Pass `--session <id>` on subsequent turns

**Proposed approach:**
- Phase 1: Omit `resume_flag` (set to `None`). Spawn a fresh subprocess per turn. Kimi's context is lost between turns, but this is the safest option.
- Phase 1.5: Parse stderr for session ID and use `--session <id>`.
- Phase 2: Use Wire mode for true persistent sessions.

---

## 11. Testing Plan

### Unit Tests

1. **Backend:**
   - `providers::get_provider("kimi")` resolves correctly
   - `providers::get_provider("kimi-cli")` resolves via alias
   - Provider list has 6 entries
   - `KIMI.controller_type` is `Subprocess`

2. **Frontend:**
   - `KimiTranslator.translate()` handles assistant text
   - `KimiTranslator.translate()` handles thinking blocks
   - `KimiTranslator.translate()` handles tool_calls with JSON string arguments
   - `KimiTranslator.translate()` handles tool results with array content
   - `createTranslator("kimi-stream-json")` returns `KimiTranslator`

### Integration Tests

1. **Stream parsing:** Feed sample Kimi NDJSON lines into `KimiTranslator` and verify `StreamEvent` output.
2. **End-to-end:** Open a Kimi agent pane, send "List files in current directory", verify tool call and result render correctly.
3. **Auth isolation:** Verify `KIMI_SHARE_DIR` is set and Kimi reads/writes to the isolated directory.

### Manual Tests

1. Run `kimi --print --output-format stream-json --yolo -p "say hi"` and capture output.
2. Run `kimi --print --output-format stream-json --yolo -p "use Shell to list files"` and verify tool call JSON is well-formed.
3. Test `--continue` in print mode for session resumption.

---

## 12. Open Questions & Risks

| # | Question / Risk | Mitigation |
|---|-----------------|------------|
| 1 | **Session resumption in print mode** — does `--continue` work with `--print`? | Test manually. If broken, omit `resume_flag` in Phase 1. |
| 2 | **System prompt injection** — Kimi doesn't read `CLAUDE.md`. How do we inject Forge `soul`/`agentmd`? | Pass as part of the initial `-p` prompt, or use `--agent-file` (Phase 2). |
| 3 | **MCP support** — Kimi uses `--mcp-config-file` not `.mcp.json` auto-discovery. | Add `--mcp-config-file` to launch args pointing to generated `.mcp.json`. |
| 4 | **Permission modes** — Kimi only has `--yolo` (all approvals) vs interactive. No granular modes. | Map all AgentMux permission modes to `--yolo` in Phase 1. Wire mode (Phase 2) enables real approvals. |
| 5 | **Tool name normalization** — Kimi uses `Shell`, `Read`, `Edit` etc. Same as Claude? | Verify tool names match. If different, add normalization in `KimiTranslator`. |
| 6 | **Installation** — No npm package. PATH fallback means no version isolation. | Accept for Phase 1. Phase 2 adds `uv tool install` support. |
| 7 | **Windows compatibility** — Kimi on Windows is `kimi.exe` (Python entry point). | `where kimi` should resolve it. The `.cmd` suffix handling in `cli_handlers.rs` may need adjustment. |
| 8 | **Stderr parsing** — Kimi prints session resume info to stderr (`To resume this session: kimi -r <id>`). | Our subprocess/persistent controllers already drain stderr. We can parse it if needed for session ID extraction. |

---

## 13. Implementation Checklist

- [ ] **Backend:** Add `KIMI` provider config to `providers.rs`
- [ ] **Backend:** Register `kimi` in `REGISTRY`, `ORDER`, and `ALIASES`
- [ ] **Backend:** Update `app_api.rs` output_format match for `kimi-stream-json`
- [ ] **Backend:** Add PATH fallback to `cli_handlers.rs` when `npm_package` is empty
- [ ] **Backend:** Update provider count in unit tests (5 → 6)
- [ ] **Frontend:** Extend `outputFormat` / `styledOutputFormat` union types in `providers/index.ts`
- [ ] **Frontend:** Add `kimi` provider definition to `PROVIDERS`
- [ ] **Frontend:** Create `providers/kimi-translator.ts`
- [ ] **Frontend:** Register `kimi-stream-json` in `translator-factory.ts`
- [ ] **Frontend:** Update `buildRuntimeArgs.ts` permission flags for Kimi
- [ ] **Frontend:** Add unit tests for `KimiTranslator`
- [ ] **Integration:** Manually test `kimi --print --output-format stream-json` end-to-end
- [ ] **Integration:** Verify auth isolation via `KIMI_SHARE_DIR`
- [ ] **Docs:** Update `app-api-status.md` if provider list is documented there

---

## 14. Appendix: Kimi NDJSON Samples

### Sample A: Simple text response
```json
{"role":"assistant","content":[{"type":"think","think":"The user wants a greeting."}],"tool_calls":null}
{"role":"assistant","content":[{"type":"text","text":"Hello! How can I help you today?"}],"tool_calls":null}
```

### Sample B: Tool call + result
```json
{"role":"assistant","content":[],"tool_calls":[{"type":"function","id":"tool_abc123","function":{"name":"Shell","arguments":"{\"command\":\"ls -la\"}"}}]}
{"role":"tool","tool_call_id":"tool_abc123","content":[{"type":"text","text":"total 128\\ndrwxr-xr-x  5 user staff   160 Apr 20 10:00 .\\n..."}]}
{"role":"assistant","content":[{"type":"text","text":"Here are the files in the directory."}],"tool_calls":null}
```

### Sample C: Thinking + text
```json
{"role":"assistant","content":[{"type":"think","think":"I need to analyze the codebase structure first."}],"tool_calls":null}
{"role":"assistant","content":[{"type":"text","text":"I'll start by examining the project structure."}],"tool_calls":null}
```

---

## 15. Appendix: Comparison with Existing Providers

| Feature | Claude | Codex | Gemini | **Kimi (Phase 1)** |
|---------|--------|-------|--------|-------------------|
| Controller | Subprocess | Subprocess | Subprocess | **Subprocess** |
| Stream format | stream-json (Anthropic deltas) | NDJSON (discrete) | stream-json (discrete) | **stream-json (discrete)** |
| Tool calls | Streaming deltas | Discrete | Discrete | **Discrete** |
| Thinking | `thinking_delta` | No | No | **`think` content part** |
| Session resume | `--resume` | `exec resume <id>` | `-r` | **`--continue` (TBD)** |
| Auth isolation | `CLAUDE_CONFIG_DIR` | `CODEX_HOME` | `GEMINI_CLI_HOME` | **`KIMI_SHARE_DIR`** |
| Install | npm | npm | npm | **PATH / pip / uv** |
| MCP | Auto `.mcp.json` | Via config | Via config | **`--mcp-config-file`** |

---
*End of Spec*
