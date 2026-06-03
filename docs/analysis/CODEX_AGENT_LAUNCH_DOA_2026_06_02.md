# Codex agent DOA — Claude-shaped runtime args leak into the codex command

**Date:** 2026-06-02
**Status:** root-caused; fix in `frontend/app/view/agent/buildRuntimeArgs.ts`
**Severity:** codex agents are **completely non-functional** (process exits <100 ms on every launch)
**Area:** agent provider arg construction (translator-unification, RFC #753 Phase 1.5)

## Symptom

Loading a Codex agent pane shows nothing. The pane mounts, but no agent
output ever appears — "I tried loading it but it's not doing anything."

## Evidence (dev instance, block `e5845d83`)

The codex subprocess is spawned and dies in the same ~80 ms window, twice:

```
subprocess spawned block_id=e5845d83 pid=25004 cmd=...\cli\codex\...\codex.cmd
  args=["exec", "--json", "--dangerously-bypass-approvals-and-sandbox", "-",
        "--dangerously-skip-permissions", "--model", "sonnet"]
subprocess stderr  block_id=e5845d83
  stderr=Usage: codex exec --json --dangerously-bypass-approvals-and-sandbox <PROMPT>
```

codex CLI v0.116.0 prints its usage banner and exits — it never emits a single
NDJSON event, so `CodexTranslator` (which is correct) has nothing to translate
and the pane stays empty.

## Root cause

`buildRuntimeArgs(baseLaunchArgs, runtime, providerId)` is written around
Claude Code's CLI conventions and applies them to every provider. Three Claude
assumptions break codex:

1. **Permission flag.** Codex isn't special-cased, so it falls into the `else`
   branch and gets `PERMISSION_FLAGS.bypass` = `--dangerously-skip-permissions`
   appended. That is a **Claude** flag. Codex's real bypass
   (`--dangerously-bypass-approvals-and-sandbox`) is already baked into its base
   args (`providers/index.ts` `launchArgs`), and `codex exec` has no
   `--permission-mode` equivalent. codex rejects the unknown flag.

2. **Model value.** `supportsModel` includes `codex`, so `--model <config.model>`
   is appended. But `ModelChoice = "opus" | "sonnet" | "haiku"` — these are
   **Claude model names**. `DEFAULT_RUNTIME_CONFIG.model = "sonnet"`, so codex is
   handed `--model sonnet`, which is not an OpenAI/codex model.

3. **Arg order.** Codex's base args **end with the positional `-`** (read prompt
   from stdin). `buildRuntimeArgs` *appends* overrides, so every override lands
   **after** the positional. codex parses `-` as the prompt and then sees
   stray flags where it expects end-of-args → usage error.

The deeper issue: `AgentRuntimeConfig` (`permissionMode` / `model` / `effort`)
and `ModelChoice` are **Claude-shaped types applied uniformly to all providers**
with no per-provider translation. The codex *output* translator was unified
(`codex-translator.ts`); the *input/launch* arg path was not.

## Fix

Make `buildRuntimeArgs` codex-aware (mirrors the existing kimi/gemini
special-case for `--yolo`):

- **Permission:** codex takes **no** appended permission flag — its bypass is in
  base args and `exec` has no `--permission-mode`.
- **Model:** `ModelChoice` values (opus/sonnet/haiku) are Claude names codex
  rejects, so codex is excluded from the `ModelChoice` `--model` path. Instead a
  current ChatGPT-account codex model is inserted **before** the `-` positional.
  `CODEX_DEFAULT_MODEL = "gpt-5.4"`.

Result — the correct, working command:

```
codex exec --json --dangerously-bypass-approvals-and-sandbox --model gpt-5.4 -
```

## Second error (surfaced after the command fix): model not allowed for ChatGPT auth

With the command corrected, codex spawned, reached the OpenAI backend, and
returned:

```
400 invalid_request_error: The 'gpt-5.3-codex' model is not supported when
using Codex with a ChatGPT account.
```

This *confirms* the command-construction fix worked — codex now launches and
connects (no more `Usage:` crash). Dropping the bogus `--model sonnet` had left
codex on its stale baked default (`gpt-5.3-codex`), which OpenAI **retired for
ChatGPT-account auth**. The ChatGPT-account codex models (June 2026) are
**gpt-5.5**, **gpt-5.4**, **gpt-5.4-mini**, and **gpt-5.3-codex-spark** (Pro-only).
We pin **gpt-5.4** (the codex flagship; also referenced in the user's own prior
session). A real per-provider model enum + dropdown (replacing the Claude-only
`ModelChoice`) remains the architectural follow-up.

## Sibling / follow-ups (state the rule, grep the siblings)

- **gemini latent bug:** `gemini` is also in `supportsModel`, so gemini agents
  receive `--model sonnet` too (a Claude name, invalid for gemini). Not fixed in
  this pass (no repro captured, and gemini may need its own model list rather
  than removal) — left as-is and tracked here so we don't silently regress it.
- **Architectural:** `AgentRuntimeConfig` / `ModelChoice` should become
  per-provider (codex/gemini get their own model enums + permission semantics),
  completing the translator-unification (RFC #753 Phase 1.5). Until then, any new
  non-Claude provider added to the `else` / `supportsModel` paths inherits this
  same class of bug.

## Verification

1. In `task dev`, launch a Codex agent.
2. Confirm in the host log the spawn args are exactly
   `["exec","--json","--dangerously-bypass-approvals-and-sandbox","-"]` — no
   `--dangerously-skip-permissions`, no `--model`.
3. Confirm **no** `Usage: codex exec ...` on stderr and that
   `thread.started` / `item.completed` events flow into the pane.
