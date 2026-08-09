# Specification: Agent Harness vs. Model Vendor Decoupling & Antigravity (AGY) Integration

**Document Version:** `1.0.0`  
**Date:** `2026-08-09`  
**Author:** Antigravity AI & AgentMux Core Team  
**Status:** Approved / In Implementation  

---

## 1. Executive Summary & Problem Statement

Historically in AgentMux (and the broader AI coding ecosystem), an **Agent Provider** conflated two distinct dimensions:
1. **Agent Harness (CLI Execution Engine / Driver):** The client-side CLI executable that drives agent turns, manages tool permissions, reads local workspace files, constructs system prompts, and handles IPC (e.g., Claude Code `claude`, Antigravity `agy`, OpenAI Codex `codex`, Gemini CLI `gemini`, OpenClaw `openclaw`, MuxCode `muxcode`).
2. **Model Vendor / Backend (Intelligence Source):** The LLM API endpoint or model family serving responses (e.g., Anthropic Claude Sonnet 5 / Opus 4.8, Google Gemini 3.6 Flash / 2.5 Pro, OpenAI GPT-5.5 / o3-mini, OpenRouter, local Ollama GGUF, or enterprise Vertex / Bedrock proxies).

This coupling caused operational limitations:
- **Claude Code with non-Anthropic / Proxy models:** Users can run Claude Code (`claude`) against custom API proxies, OpenRouter, or Bedrock (`ANTHROPIC_BASE_URL`).
- **Antigravity CLI (`agy`):** Google's agentic harness (`agy`) drives high-throughput tasks using Gemini 3.6 Flash, Gemini 2.5 Pro, or custom DeepMind models, with native skill discovery, MCP integration, and subagent orchestration.

This specification establishes an explicit **Harness vs. Model Vendor** architecture in AgentMux and introduces **Antigravity (`agy`)** as a first-class supported provider harness.

---

## 2. Architecture & Conceptual Model

```
+-----------------------------------------------------------------------------------+
|                                  AGENT MUX                                        |
+-----------------------------------------------------------------------------------+
                                        |
                    +-------------------+-------------------+
                    |                                       |
                    v                                       v
    +-------------------------------+       +-------------------------------+
    |         AGENT HARNESS         |       |      MODEL / BACKEND VENDOR   |
    |      (Execution Engine)       |       |     (Intelligence Endpoint)   |
    +-------------------------------+       +-------------------------------+
    | * Executable CLI / Driver     |       | * Provider Vendor / Endpoint  |
    |   - Claude Code (`claude`)    |       |   - Anthropic (Direct/Vertex) |
    |   - Antigravity (`agy`) [NEW] |       |   - Google (AI Studio/Vertex) |
    |   - Codex (`codex`)           |       |   - OpenAI / Azure OpenAI     |
    |   - Gemini CLI (`gemini`)     |       |   - OpenRouter / LiteLLM      |
    |   - OpenClaw (`openclaw`)     |       |   - Local Ollama / vLLM       |
    |   - MuxCode (`muxcode`)       |       |   - Custom Proxy / Bedrock    |
    | * Execution Protocol          |       | * Authentication & Keys       |
    |   - Persistent / Stream-JSON  |       |   - API Keys, OAuth tokens    |
    |   - Subprocess / ACP          |       |   - Base URL & Env overrides  |
    | * Tool & Subagent System      |       | * Selected Model Identifier   |
    +-------------------------------+       +-------------------------------+
                    \                                       /
                     \                                     /
                      v                                   v
             +-------------------------------------------------+
             |              RUNNING AGENT SESSION              |
             |       Harness(agy) + Model(gemini-3.6-flash)   |
             |       or Harness(claude) + Model(openrouter)    |
             +-------------------------------------------------+
```

### 2.1 The Dimensions Defined

| Dimension | Responsibilities | Key Configuration Attributes |
| :--- | :--- | :--- |
| **Agent Harness** | Terminal spawn, stdin/stdout stream parsing, tool permission UI, subagent management, session resume | `cli_command`, `controller_type`, `styled_output_format`, `launch_args`, `resume_flag`, `session_id_field` |
| **Model Vendor** | Upstream API host, authentication credentials, model catalog selection, base URL routing | `auth_type`, `auth_config_dir_env_var`, `base_url_env_var`, `api_key_env_var`, `models` |

---

## 3. Antigravity (`agy`) Integration Details

Google's **Antigravity CLI (`agy`)** is integrated as a high-performance subprocess provider harness.

### 3.1 Provider Configuration Matrix

```rust
static ANTIGRAVITY: ProviderConfig = ProviderConfig {
    id: "antigravity",
    cli_command: "agy",
    controller_type: ControllerType::Subprocess,
    launch_args: &["--output-format", "stream-json", "--yolo", "-p", ""],
    persistent_launch_args: None,
    resume_flag: Some("-r"),
    session_id_field: "session_id",
    styled_output_format: "gemini-json",
    auth_config_dir_env_var: "ANTIGRAVITY_CONFIG_DIR",
    auth_dir_name: "antigravity",
    auth_extra_env: &[("ANTIGRAVITY_FORCE_FILE_STORAGE", "true")],
    unset_env: &[],
    npm_package: "@google/antigravity-cli",
    pinned_version: "1.0.0",
    harness: HarnessEngine::Agy,
    supported_vendors: &[ModelVendor::Google, ModelVendor::Custom],
};
```

### 3.2 Model Catalog for Antigravity

- `gemini-3.6-flash` (Default, Ultra-low latency, 1M context)
- `gemini-2.5-pro` (Reasoning & complex agent tasks)
- `gemini-2.5-flash` (Balanced)
- `gemini-2.0-flash-thinking` (Deep chain-of-thought)
- `flash_lite` (Fast lightweight tasks)

---

## 4. Harness vs. Model Matrix in AgentMux

| Provider ID | Harness Engine (`cli_command`) | Primary Vendor | Supported Alternative Vendors | Environment Overrides |
| :--- | :--- | :--- | :--- | :--- |
| `claude` | `claude` (Claude Code) | Anthropic | OpenRouter, Bedrock, Vertex, Custom | `ANTHROPIC_BASE_URL`, `CLAUDE_CONFIG_DIR` |
| `antigravity` | `agy` (Antigravity CLI) | Google | Vertex AI, Custom Proxy | `GEMINI_CLI_HOME`, `ANTIGRAVITY_CONFIG_DIR` |
| `codex` | `codex` (Codex CLI) | OpenAI | Azure OpenAI, Custom Proxy | `CODEX_HOME`, `OPENAI_BASE_URL` |
| `gemini` | `gemini` (Gemini CLI) | Google | Vertex AI, Custom Proxy | `GEMINI_CLI_HOME` |
| `qwen` | `qwen` (Qwen Code) | OpenRouter / Alibaba | OpenAI-compatible proxies | `QWEN_HOME`, `OPENAI_BASE_URL` |
| `muxcode` | `muxcode` (Mux Code) | Local GGUF / Multi | Anthropic, OpenAI, Custom | `MUXCODE_CONFIG_DIR` |
| `openclaw` | `openclaw` (OpenClaw ACP) | Model-Agnostic | Pi, OpenAI, Claude, Gemini | `OPENCLAW_HOME` |

---

## 5. Verification & Backward Compatibility

1. **Alias Canonicalization:** Legacy and shorthand provider strings (`agy`, `antigravity-cli`, `claude-code`, etc.) resolve seamlessly to canonical provider definitions in both Rust (`backend/providers.rs`) and TypeScript (`provider-id-aliases.ts`).
2. **Existing Agent Sessions:** Existing workspace databases and agent definitions retain full compatibility; default harness values automatically map to historical provider defaults.
