# AgentMux Token Tax — Follow-up: P1–P4 Resolution + Empirical Cache Check

**Date:** 2026-07-04
**Follow-up to:** `docs/analysis/TOKEN_TAX_ANALYSIS_2026_06_19.md`
**Author:** AgentA

Closes out the four open items (P1–P4) from the original analysis and adds an empirical cache-behavior test that wasn't in the original (desk-research-only) doc. Also folds in a second, distinct overhead vector — MCP tool-schema size — that came up in the same investigation but isn't covered by the original doc at all.

---

## 1. Status of P1–P4

| Item | Original status | Now |
|---|---|---|
| **P1** — default `startup` to `__SKIP__` | Proposed | **Already shipped.** `scripts/gen-seed.js:270-272` defaults `startup: "__SKIP__"`. No action needed. |
| **P2** — dedupe CLI auto-memory vs. AgentMux CLAUDE.md memory | Open (3 options listed, undecided) | **Already resolved — Option A shipped.** `agentmux-srv/src/server/app_api/agent_open.rs:418-436` defaults `autoMemoryEnabled: false` in `.claude/settings.json` whenever the file's settings block is (re)written, unless the user already set it explicitly. AgentMux owns memory itself via CLAUDE.md's global-brain + per-agent memory injection (`agent_config.rs`). The two-memory-systems risk the doc flagged doesn't exist today. |
| **P3** — does `--resume` re-read CLAUDE.md from disk? | Unverified hypothesis ("logical answer is no") | **Empirically tested — resolved, with a more precise answer than the original hypothesis (see §2).** |
| **P4** — add `--exclude-dynamic-system-prompt-sections` | Proposed | **Shipped this session.** Added to both `launch_args` and `persistent_launch_args` for the `claude` provider in `agentmux-srv/src/backend/providers.rs`. Verified the flag is real (`claude --help`), verified AgentMux doesn't pass `--system-prompt` anywhere (which would make the flag a no-op), verified `cargo check -p agentmux-srv` compiles clean. |

---

## 2. P3 — empirical test (three-turn controlled experiment)

Ran directly against the `claude` CLI in a scratch directory (`/tmp/claude-resume-test`), independent of AgentMux, to isolate the CLI's own `--resume` mechanics:

1. **Turn 1** (fresh session): `CLAUDE.md` contains `SENTINEL_ORIGINAL_VALUE_ALPHA`. Asked "what does CLAUDE.md say" → correctly quoted `ALPHA`.
2. Edited `CLAUDE.md` on disk to `SENTINEL_CHANGED_VALUE_BETA`.
3. **Turn 2** (`--resume <session_id>`, explicit "what does it say **now**"): correctly quoted `BETA`. `num_turns: 2` in the response — i.e. the model made a tool call (a `Read`/file-check) to verify before answering, it wasn't handed fresh content automatically.
4. Edited `CLAUDE.md` again to `SENTINEL_THIRD_VALUE_GAMMA`.
5. **Turn 3** (`--resume`, explicitly "recall from memory, don't re-read anything"): the model answered `ALPHA` then `BETA` (its own conversation history) and did **not** mention `GAMMA`. `num_turns: 1` — no tool call.

**Conclusion:** the original doc's hypothesis was directionally right but for the wrong reason. `--resume` does **not** mechanically re-inject a fresh copy of `CLAUDE.md` into context on every resume (turn 3 proves this — no re-read, no `GAMMA`). But `CLAUDE.md` is just an ordinary file in the working directory, so **the agent can always see a mid-session edit if something (a user question, a prompt, its own judgment) causes it to actually read the file** — that's turn 2. The doc's practical concern (does AgentMux writing memory into `CLAUDE.md` mid-session become visible before the next fresh session?) resolves to: **not automatically, but not structurally blocked either** — it depends on whether the running agent happens to check the file. In practice, for AgentMux's actual usage (memory writes aren't something the agent is routinely prompted to re-verify), the doc's original practical conclusion holds: **treat CLAUDE.md memory writes as effectively cross-session-only.**

### Bonus finding: the Anthropic baseline cache really is warm cross-session

Turn 1 (a nominally "fresh" session, first invocation) already showed `cache_read_input_tokens: 44661` against `cache_creation_input_tokens: 6212`. That's a *new* session reading tens of thousands of tokens from cache before it had ever run — confirming the original doc's claim that the ~27–32K Anthropic system-prompt-plus-tools baseline is cached by content hash and shared across sessions, not per-session. This is good news for the overhead story: the biggest cost line item in the whole breakdown is already close to free after the very first request anywhere on the account.

---

## 3. P4 — shipped

`agentmux-srv/src/backend/providers.rs`, `CLAUDE` provider config — added `--exclude-dynamic-system-prompt-sections` to both `launch_args` (one-shot subprocess mode) and `persistent_launch_args` (the mode AgentMux actually runs, per `controller_type: ControllerType::Persistent`).

Verified before shipping:
- The flag is real: `claude --help` → *"Move per-machine sections (cwd, env info, memory paths, git status) from the system prompt into the first user message. Improves cross-user prompt-cache reuse. Only applies with the default system prompt (ignored with `--system-prompt`). (default: false)"*.
- It won't be silently ignored: grepped the whole `agentmux-srv` tree for `--system-prompt` / `--append-system-prompt` — AgentMux never passes either, so the default system prompt path applies and the flag takes effect.
- `cargo check -p agentmux-srv` — clean (pre-existing dead-code warnings only, unrelated to this change).

Effect: per-machine content (cwd, git status, env info) moves out of the cached system-prompt prefix into the first user message, so two different AgentMux instances (or the same instance across working directories) share the same system-prompt cache entry instead of each minting its own.

---

## 4. A second, distinct overhead vector: MCP tool-schema size (not covered by the original doc)

Separately from CLAUDE.md/session-start overhead, this session also dug into whether `agentmux-mcp`'s tool definitions (28 tools) get "resent every turn" — they do, but that's inherent to the stateless Messages API (every tool, MCP or not, must ride in the `tools` array on every request), not something specific to MCP or fixable by AgentMux. What *is* controllable:

- Tools render **before** `system` in the cache prefix (`tools → system → messages`), so a stable tool set is cache-eligible the same way the system prompt is — same 1hr TTL for OAuth, same cross-request reuse.
- Per the existing `SPEC_AGENT_API_FIRST_CLASS_SURFACE_2026_06_17.md` §10 amendment, the MCP tool surface was already consolidated 17 → 11 tools specifically to shrink this footprint (and to stay under the 4-breakpoint cache_control limit's practical concerns).
- That amendment's own citation (`agentmux-pane-latency-report.md`) **does not exist in this repo** — flagging this again since it means the "2 KB per turn" cost claim behind that consolidation was never independently verified against real `cache_read_input_tokens` telemetry, only assumed. This session's P3 experiment is the first actual empirical cache measurement done against this codebase's usage pattern.
- AgentMux does **not** persist `cache_read_input_tokens`/`cache_creation_input_tokens` anywhere queryable (checked `agentmux-srv/src/backend/storage/` — no such column/table). The frontend (`claude-translator.ts`, `useAgentStream.ts`) parses these fields per-turn for the live cost/token counters, but nothing rolls them up historically. **There's no way today to check "is caching actually working" for a real AgentMux agent session after the fact** — only live, in the moment, in the UI.

---

## 4b. Addendum — the actual `agentmux-docs` page (not checked when this report was first written)

This report initially only used `docs/analysis/TOKEN_TAX_ANALYSIS_2026_06_19.md`, an internal analysis doc inside *this* repo. There is a separate, authoritative public docs repo, `agentmuxai/agentmux-docs` (docs.agentmux.ai) — cloned read-only afterward to check `src/content/docs/internals/conversation-overhead.md` directly. It corroborates everything above (two-layer model, `cache_control: ephemeral`, the `input + cache_creation + cache_read` formula) and adds facts worth folding in:

- **Explicit cache-invalidation trigger list** (any of these forces the next turn-1 to pay full `cache_creation_input_tokens` again): soul/agentmd/memory-bundle content edited, a memory bundle added/removed, the skills index changing (skill installed/removed), a new session started without `--resume`, or the working directory changing (it feeds `{{WORKING_DIR}}` template substitution into the assembled CLAUDE.md). Anything touching P2's memory-bundle system is therefore also a cache-cost lever, not just a correctness one.
- **AgentMux makes zero direct HTTP calls to any AI provider API** — all provider interaction is through the CLI subprocess via PTY. This bounds what AgentMux can ever do about per-turn overhead to: (a) what content it writes into CLAUDE.md, (b) CLI launch flags (P4's category), and (c) session lifecycle (fresh vs. `--resume`) — never direct `cache_control` placement, since AgentMux never builds the API request itself.
- **Compaction threshold for the Claude Code CLI: `contextWindow - 33,000` tokens.** AgentMux detects compaction by watching for a token-count drop in the CLI's output stream but doesn't trigger or control it.
- One stale citation in that page: it cites `agentmux-srv/src/backend/types.rs` for `TokenCounts` — the struct actually now lives at `agentmux-srv/src/agents/types.rs` (confirmed by grep; the file moved since that docs page was written). Same pattern as the stale `agent_handlers.rs` citation in the original 2026-06-19 analysis doc — worth a docs pass to re-anchor citations against current file paths.
- Confirms independently (via `struct TokenCounts` in `agents/types.rs` and a repo-wide grep of `backend/storage/`) that this data is **not persisted anywhere** — same conclusion §4 already reached, now cross-checked against the public docs' own claim that AgentMux "tracks" these fields (true only in the sense of parsing them into a live struct for the UI, not storing history).

## 4c. Code-verification pass — every claim checked against the current source

Both docs pages (the internal 2026-06-19 analysis and `agentmux-docs`' `conversation-overhead.md`) cite specific files/functions. Rather than trust those citations, verified each one directly against `main` @ `ca7ff4d3`:

| Claim | Result |
|---|---|
| CLAUDE.md assembly (Soul + AgentMD + memory + skills + template vars) | **Confirmed**, citation stale. Real path: `agentmux-srv/src/backend/agent_config.rs:35` `build_config_files(...)`, called from `agentmux-srv/src/server/app_api/agent_open.rs:393` `write_agent_config_files(...)` — not `agentmux-srv/src/server/app_api.rs` as cited. Assembly order confirmed: Soul → `---` → AgentMD → `# Memory` (per-agent + global brain bundles) → `# Available Skills`. |
| Written once per launch, not per turn | **Confirmed.** Single call site: `agent_open.rs:321`, inside the `agent.open` RPC handler. No other caller anywhere in the tree. |
| Startup payload (fresh-session-only, contents) | **Confirmed**, citation stale. Real path: `frontend/app/view/agent/startup/buildStartupPayload.ts` (moved into a `startup/` subfolder), invoked from `agent-view.tsx:654`, guarded at line 635 by `if (block()?.meta?.["agent:sessionid"]) return;` (skips on resume). |
| `real_context_tokens = input + cache_creation + cache_read` | **Confirmed**, citation stale. Real path: `frontend/app/view/agent/providers/claude-translator.ts:95-98` (moved into a `providers/` subfolder). The Rust struct (`agentmux-srv/src/agents/types.rs:112`, not `backend/types.rs`) uses shortened field names (`input`, `cache_creation`, `cache_read`) — the `_input_tokens`-suffixed wire names only exist in the frontend's raw parsing. |
| Compaction threshold `contextWindow - 33,000` | **Confirmed**, and better-anchored than either doc: `frontend/app/store/agent-pane-state/context-window.ts:26` — `const COMPACTION_BUFFER = 33_000;`. Neither doc actually cited this file. The file's own header explains *why*: the CLI never reports its context window size; AgentMux learns/seeds it from observed usage. The specific "large token-count drop" detection logic referenced by the internals doc was not separately located — flagging as not fully verified (the threshold/meter math is confirmed, the drop-detector isn't pinned to a line). |
| Cache-invalidation triggers all resolve through the same launch-time CLAUDE.md rewrite | **Confirmed for 4 of 5 — 1 is currently false.** Soul/agentmd/memory edits, memory-bundle add/remove, and skills-index changes are all read fresh inside `write_agent_config_files` and folded into the same single call site as above — correct. **But "working directory changes" does not currently invalidate anything**: `build_config_files` (the function actually called) explicitly does *not* set the `{{WORKING_DIR}}` template var (`agent_config.rs:51-52`: *"WORKING_DIR is not available in this signature; leave it empty... expansion will leave `{{WORKING_DIR}}` intact if absent"*). The sibling function that does set it, `build_config_files_with_bus`, **has zero callers anywhere in the codebase** — confirmed via a repo-wide grep excluding its own definition. So today, changing working directory has no effect on CLAUDE.md content and therefore doesn't invalidate the cache the way `agentmux-docs` currently claims. This is either dead code that should be wired up, or a docs claim that should be removed — flagging both possibilities rather than picking one, since fixing it either way is outside this session's scope. |

## 5. Open items / next steps

1. **No historical cache-hit telemetry.** If overhead work continues, the highest-leverage next step is persisting `cache_read_input_tokens` / `cache_creation_input_tokens` (already parsed, just not stored) per turn, so "is the cache actually landing for real agent sessions" becomes a query instead of a one-off manual test like this session's.
2. **P4's actual effect is unmeasured.** The flag is shipped but its real-world cache-reuse improvement hasn't been measured before/after (needs #1 above, or a manual two-instance comparison).
3. **Confirm P2 in practice, not just in code.** `autoMemoryEnabled: false` is the default *write*, but worth spot-checking a live agent's actual `.claude/settings.json` to confirm it's landing (the code path warns and skips the guard if the existing `settings.json` is unparseable JSON — `agent_open.rs:428-436` — worth checking that warning never fires in practice).
4. **The missing `agentmux-pane-latency-report.md` citation** — either the file should exist and was lost, or the Amendment's cost claim should be re-derived from real data (see #1) and the citation corrected.
5. **`{{WORKING_DIR}}` templating is dead code.** `build_config_files_with_bus` (the only function that substitutes it) has no callers. Decide: wire it into `write_agent_config_files` (making "working directory changed" a real cache-invalidation trigger, matching what `agentmux-docs` already claims), or delete the dead function and correct the docs claim. Either is a small, self-contained follow-up.
6. **Stale file-path citations, repo-wide.** Two separate docs (the internal 2026-06-19 analysis and `agentmux-docs`' `conversation-overhead.md`) both cite paths that have since moved (`app_api.rs` → `app_api/agent_open.rs`, `agent/buildStartupPayload.ts` → `agent/startup/buildStartupPayload.ts`, `agent/claude-translator.ts` → `agent/providers/claude-translator.ts`, `backend/types.rs` → `agents/types.rs`). Fixed in `agentmux-docs` directly (see §6).

## 6. `agentmux-docs` update

Once this report's changes land on `main`, the corresponding public-docs PR against `agentmuxai/agentmux-docs` updates `src/content/docs/internals/conversation-overhead.md` with: the four corrected file-path citations from §4c, the `{{WORKING_DIR}}` dead-code caveat on the "working directory changes" invalidation trigger (open item #5 above — noted as currently inert rather than silently removed, since the fix direction isn't decided yet), and a new citation for the compaction-threshold constant (`context-window.ts:26`), which neither doc had pinned to a file before.
