# Report: nonsensical "next prompt" ghost text, and what the ambient calls duplicate

**Status:** analysis
**Date:** 2026-10-01
**Author:** agent1
**Trigger:** the repo owner: the Haiku "next prompt" suggestion is sometimes nonsense and
sometimes sensible but inappropriate. Example shown in the composer:

> I don't have access to the details of the recent activity beyond the cost summary.
> Without knowing what work was done or what the current state of the project is, I cannot
> plausibly predict the next instruction. If you'd like me to predict your next step, I'd
> need to see the actual recent work or conversation history.

## 1. What happens today

`session:next_prompt_suggestion` (`crates/srv/src/server/app_api/session.rs`):

1. `read_recent_activity_digest` reads the tail of the block's `output` file, keeps the
   **last 30 non-empty raw lines**, and turns them into text with `extract_digest_text`.
2. It sends that to `claude -p --model claude-haiku-4-5-20251001` as one prompt.
3. Whatever text comes back, after fence/quote/"Yeah, let's" stripping, is returned as the
   suggestion. The frontend writes it to `term:next_prompt_suggestion` and the composer shows
   it as ghost text.

## 2. Root causes

| # | Cause | Evidence |
|---|---|---|
| 1 | **The digest can contain no conversation.** The window is 30 raw JSON lines; on a streaming session most are `stream_event` deltas, which the extractor skips. `result` lines are turned into `[summary] 3 turns, $0.0412 total cost`, and that alone makes the digest non-empty. | The reported text names "the cost summary" as the only thing it was given. |
| 2 | **Haiku runs as a full Claude Code assistant.** The call keeps the default system prompt, tools, slash commands, MCP servers and session persistence. A thin prompt gets an assistant's answer ("I don't have access…"). | Live run with the old flags and a cost-only digest answers in the assistant's voice; with `--tools ""`, a plain system prompt and one turn it returns an empty fence, which sanitizes to nothing. |
| 3 | **Nothing validates the output.** `sanitize_ambient_text` only removes formatting and "Yeah/Sure/Let's…" openers. A refusal, a question, a paragraph or a placeholder is shown as is. | The reported text passed through unchanged. |
| 4 | **The prompt gives the model no way to say "nothing".** It asks for a prediction, offers an empty string only as a vague last resort, and says nothing about when a suggestion is wrong. | "Sensible but inappropriate": it suggests a next step right after the assistant asked the user a question, or after the work is finished, or something destructive. |
| 5 | The digest keeps no more than the last 120 chars of an error and nothing of tool inputs, so the model often can't tell what the work is. | `extract_digest_text` emits `[tool] name` only. |

The frontend (`useNextPromptSuggestion.ts`) is not at fault: it already guards against
stale turns, a non-empty composer and hidden turns. It just trusts whatever the backend
returns.

## 3. Fix (this PR)

- **Digest:** drop the `[summary]` cost line (it is not conversation). Read a 96 KB tail and
  extract from all of it instead of the last 30 lines, keep the newest 14 entries within 6,000
  characters (each entry clipped to 700, keeping its end), and return no digest unless it holds
  at least one `[user]` or `[assistant]` entry. No digest means no model call at all.
- **Call:** `--tools ""`, `--max-turns 1`, `--no-session-persistence`,
  `--disable-slash-commands`, `--strict-mcp-config`, and our own system prompt
  ("a text-generation function, not a conversational assistant… if the instruction cannot be
  met, output nothing"). Applies to every ambient call.
- **Prompt:** the activity goes in a tagged block; the model is told to output nothing when the
  activity doesn't show the work, the assistant's last message asks the user something, or the
  work looks finished; and never to suggest deleting data, force-pushing or touching credentials.
- **Validation** (`ambient::validate::accept_next_prompt`): reject empty/placeholder text,
  multi-line text, more than 160 characters or 28 words, questions, the model talking about the
  task ("I don't have…", "If you'd like me to…", "recent activity", "next instruction"), and
  risky commands (`rm -rf`, `--force`, `reset --hard`, secrets, tokens…). A rejected suggestion
  costs nothing: the tokens are still recorded, and no ghost text is shown.

Live check with the new flags and prompt: a digest of a fixed bug with no test yet gives
"Add a regression test for the login submit handler's null check."; a digest ending in the
assistant asking "Which do you prefer?" gives nothing.

## 4. DRY findings

| Duplication | Where | Status |
|---|---|---|
| The same plain-text boilerplate ("no markdown, no code fences, no backticks, no quotes, no preamble") written out in 5 prompts, each slightly different | `session.rs` (activity summary, definition summary, subagent name, dispatch name, next prompt) | **Done here:** `ambient::prompt::PLAIN_TEXT_RULES` + `with_material` |
| Material pasted after "Recent activity:" with no delimiter, so the model can read it as a message to answer | same | **Done here:** tagged block |
| Nothing shared decides whether an ambient call's *output* is usable; each caller did its own `trim()`/`is_empty()` | 6 call sites | **Done (follow-up PR):** `ambient::validate::accept_line` runs on every reply, with per-purpose size limits; next prompts add the question/meta/risky checks |
| The CLI call, sanitizer, digest reader and 6 prompt builders lived in the 2,400-line RPC file | `session.rs` | **Done (follow-up PR):** moved to `ambient/{cli,sanitize,digest,prompt}.rs`; `session.rs` keeps handlers only (~570 lines) |
| Each caller repeated the admit / permit / invoke / drop-guard / empty-check sequence | 6 call sites | **Done (follow-up PR):** `ambient::call::admit` + `Slot::run`; the semaphores are in `ambient::limits`, the purpose tags in `ambient::purpose` |

## 5. Not changed

- The frontend hook: nothing to fix there.
- The continuity-state summarizer (`backend/continuity_state.rs`) shares the new CLI flags and
  system prompt, since it uses the same invoker. Its prompt and timeout are unchanged.
