# SPEC: Add Qwen Code & aider as agent providers

**Date:** 2026-06-02
**Status:** Draft
**Owner:** sidecar (`agentmux-srv/src/backend/providers.rs` + `agents/translator/`) + provider seed
**Scope:** Register two new harness providers — **Qwen Code** and **aider** — alongside the existing Claude/Codex/Gemini/Kimi/OpenClaw/Pi/Copilot set. Both can run **OpenRouter (or any OpenAI-compatible gateway) as their LLM backend**.

---

## 0. Framing — harness vs. LLM gateway

These are **harnesses** (agent loops), not gateways. The **LLM gateway** is OpenRouter (hosted) or LiteLLM (self-hosted); a harness points its `base_url` at the gateway:

```
harness (Qwen Code | aider | …)  →  LLM gateway (OpenRouter / LiteLLM)  →  model
```

Adding a harness here is a `ProviderConfig` entry (how to launch + auth + stream). Selecting OpenRouter as its backend is a separate per-agent config concern (identity/memory bundles) — see `SPEC_AGENT_RESEARCH_OBSERVABILITY_2026_06_02.md` §0 and the OpenRouter integration report. **This spec only adds the harnesses.**

Both verified OpenAI-compatible against OpenRouter (`https://openrouter.ai/api/v1`): Qwen via `OPENAI_BASE_URL`/`OPENAI_API_KEY`/`OPENAI_MODEL` ([Qwen model-providers](https://qwenlm.github.io/qwen-code-docs/en/users/configuration/model-providers/)); aider via `--model openrouter/<provider>/<model>` + `OPENROUTER_API_KEY` ([aider OpenRouter](https://aider.chat/docs/llms/openrouter.html)).

---

## 1. Qwen Code — clean fit ✅ (ship this)

**What it is:** an open-source terminal coding agent, a **fork of Gemini CLI** ([github.com/QwenLM/qwen-code](https://github.com/QwenLM/qwen-code)). Because it's a Gemini-CLI fork, it mirrors the existing `GEMINI` `ProviderConfig` almost exactly.

**Verified headless invocation** ([Headless Mode docs](https://qwenlm.github.io/qwen-code-docs/en/users/features/headless/)): `-p`/`--prompt` (non-interactive), `--output-format text|json|stream-json`, `--include-partial-messages` (requires stream-json), `--yolo` / `--approval-mode=yolo`. Install: `npm i -g @qwen-code/qwen-code@latest`; CLI command `qwen`.

**OpenRouter backend** (verified): `OPENAI_API_KEY=<or-key>`, `OPENAI_BASE_URL=https://openrouter.ai/api/v1`, `OPENAI_MODEL=qwen/qwen3-coder` (or via `~/.qwen/settings.json`). Note: the Qwen OAuth free tier was discontinued 2026-04-15 — OpenRouter is now a recommended path.

### 1.1 Proposed `ProviderConfig` (mirrors `GEMINI`)

```rust
static QWEN: ProviderConfig = ProviderConfig {
    id: "qwen",
    display_name: "Qwen Code",
    cli_command: "qwen",
    controller_type: ControllerType::Subprocess,
    // Gemini-CLI fork: same stream-json headless surface.
    launch_args: &["--output-format", "stream-json", "--yolo", "-p", ""],
    persistent_launch_args: None,
    resume_flag: None,              // ⚠️ VERIFY: docs mention --resume <id>/--continue; leave None until confirmed
    session_id_field: "session_id", // ⚠️ VERIFY against actual stream-json init event
    styled_output_format: "gemini-json", // reuse Gemini translator (fork) — ⚠️ VERIFY format identical
    auth_config_dir_env_var: "QWEN_CODE_HOME", // ⚠️ MUST-VERIFY (see §1.2) — not documented
    auth_dir_name: "qwen",
    auth_extra_env: &[],
    unset_env: &[],
    npm_package: "@qwen-code/qwen-code",
    pinned_version: "latest",
    icon: "feather",
    docs_url: "https://qwenlm.github.io/qwen-code-docs",
};
```

Registry + alias + test:
```rust
m.insert(QWEN.id, &QWEN);                       // in the LazyLock registry map
m.insert("qwen-code", "qwen");                  // alias map
m.insert("qwen3-coder", "qwen");                // alias map
// ORDER array: add "qwen" (e.g. after "kimi")
// test: assert get_provider("qwen").is_some() && controller_type == Subprocess
```

### 1.2 The one real blocker — config-dir isolation ⚠️
AgentMux gives each agent an isolated config/auth dir via `auth_config_dir_env_var` (e.g. Gemini's `GEMINI_CLI_HOME` + `GEMINI_FORCE_FILE_STORAGE=true`). **Qwen Code does not document an equivalent override** (defaults to `~/.qwen`). Two agents would otherwise share `~/.qwen` — an isolation bug AgentMux explicitly avoids. **Pre-merge:** confirm one of —
1. Qwen honors a `QWEN_*`/`QWEN_CODE_HOME` override (likely, given the fork — check source), **or**
2. it inherits Gemini's `GEMINI_CLI_HOME`, **or**
3. fall back to a per-agent `HOME` override (AgentMux already does this for dev isolation) + seeding `settings.json` into that dir.

This is the single field that must be right before merge; everything else mirrors a proven provider.

**Effort: S.** **Risk: low** (one verify item). High value — adds the entire Qwen3-Coder family + any OpenRouter model behind a structured, observable harness.

---

## 2. aider — poor structural fit ⚠️ (needs controller work; don't ship as a plain entry)

**What it is:** a mature Python terminal coding assistant ([aider.chat](https://aider.chat/docs/)). It's excellent, but it breaks **three** core `ProviderConfig` assumptions:

| Assumption (in `ProviderConfig`) | aider reality | Consequence |
|---|---|---|
| **Structured streaming** (`stream-json`/ACP) drives the per-provider `Translator` | aider emits **human terminal text only** — no JSON event stream ([scripting docs](https://aider.chat/docs/scripting.html)) | No rich tool-call/observability events. AgentMux would see it like a Terminal pane, not an Agent pane. |
| **npm install** via `npm_package` | aider is **pip/uv** installed (`aider-chat`), not npm | `npm_package: ""` (like `KIMI`) works as a signal, but auto-install path differs |
| **Prompt written to stdin** | aider takes the prompt as an **arg** (`--message`/`-m`) or `--message-file`, not stdin | The current Subprocess launcher (prompt→stdin) doesn't fit; needs arg/file injection |

**Verified non-interactive use:** `aider --message "…" --yes --no-stream --no-auto-commits <file>` ([scripting](https://aider.chat/docs/scripting.html)); OpenRouter via `--model openrouter/<provider>/<model>` + `OPENROUTER_API_KEY`, or generic `OPENAI_API_BASE`+`OPENAI_API_KEY`+`--model openai/<name>` ([openai-compat](https://aider.chat/docs/llms/openai-compat.html)).

### 2.1 Options
- **Option A — Text/terminal-class provider (degraded observability).** Add a `ControllerType` path that runs aider as a subprocess, passes the prompt via `--message`, captures raw stdout (a `styled_output_format: "text"` passthrough with no structured Translator). aider still *works* (edits + commits files), but AgentMux can't surface tool calls — it's "watch the terminal," not "watch the agent." Requires: an arg-prompt launch path + a no-op/text translator + a non-npm install note.
- **Option B — Defer.** Add aider only once a generic **text-translator** + **arg-prompt launcher** + **pip-install** path exist (useful for any non-structured CLI, not just aider).

### 2.2 Recommendation
**Defer aider to its own PR (Option A as the target).** Adding it as a normal `ProviderConfig` now would either silently break (stdin prompt) or present a broken Agent pane (no events). It's worth doing — but it's *launcher/controller* work, not a registry line. Track as a follow-up: "Text-class (non-streaming) provider support → enables aider, and any OpenAI-compatible REPL CLI."

---

## 3. Changes & sequencing

| PR | Change | Effort | Risk |
|---|---|---|---|
| **1** | Add `QWEN` to `providers.rs` (registry + alias + ORDER + test); reuse `gemini-json` translator; confirm §1.2 config-dir isolation | **S** | low (1 verify item) |
| **2** (later) | `ControllerType` text/non-stream path + arg-prompt launcher + pip-install support → add **aider** (Option A) | **M–L** | med (launcher surface) |

Both: add a changeset (`task changeset -- minor "feat(providers): add Qwen Code"`), **not** a `bump` (AgentMux uses changesets in feature PRs).

---

## 4. Pre-merge verify checklist (Qwen)
- [ ] **Config-dir isolation env var** (§1.2) — the only blocker.
- [ ] `resume_flag` — does `qwen` accept `--resume <id>` / `--continue`? (set or keep `None`)
- [ ] `session_id_field` — confirm the field name in the stream-json init event.
- [ ] Translator — confirm Qwen's `stream-json` is byte-compatible with Gemini's (`gemini-json`); if not, add a `qwen` translator.
- [ ] `icon` is cosmetic; pick a final glyph.

## Sources
Qwen: [README/install](https://github.com/QwenLM/qwen-code) · [headless mode](https://qwenlm.github.io/qwen-code-docs/en/users/features/headless/) · [model-providers (OpenRouter)](https://qwenlm.github.io/qwen-code-docs/en/users/configuration/model-providers/). aider: [docs](https://aider.chat/docs/) · [scripting](https://aider.chat/docs/scripting.html) · [OpenRouter](https://aider.chat/docs/llms/openrouter.html) · [OpenAI-compatible](https://aider.chat/docs/llms/openai-compat.html). OpenRouter OpenAI-compat base_url verified in `reports/openrouter-agentmux-integration-2026-06-02.md`.
