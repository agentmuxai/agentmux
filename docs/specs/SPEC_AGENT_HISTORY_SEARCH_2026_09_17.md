# SPEC: agent-facing conversation history search

**Date:** 2026-09-17
**Status:** proposed — nothing has shipped. Verified against `agentmux` @ `3fa127fbc`.
**Trigger:** Repo owner, after watching this gap cause three concrete failures
in one session (§1.1). "do agents have a streamlined way to search their
history?" — they do not.
**Builds on:** `backend/history/` (`SessionIndex`, `HistoryAdapter`,
`ClaudeHistoryAdapter`) — the discovery and parsing layer already exists and is
already used by `bundle.export_for_agent_with_history`; this adds the missing
verb on top of it, not a new subsystem.
**Related:** `SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`
and `SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md` (the
already-shipped machinery governing one agent reading *another's*
conversation — §5 explains why Phase 1 deliberately does not touch it),
`SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md` (how a
session is attributed to an identity, which is what makes "my own history"
resolvable).

---

## 1. Problem

An agent cannot search its own past. There is no verb for it anywhere in the
App API.

What exists, and why each one doesn't answer the question:

| Surface | Gives | Searchable |
|---|---|---|
| `GetAgentTranscript` | the **tail** of one agent's live session (`max_lines`, server-capped 500) | **No** — no query parameter exists |
| `ListConversations` | liveness plus a one-line preview per agent | **No** |
| `muxlog … grep <regex>` | srv / host / frontend **logs** | Yes, but **not conversation content** — `-a` adds only the sidecar's `…→ blockfile` lines |
| `muxspect` | live process/controller state | N/A — not history |
| Personal Memory | notes the agent chose to write | only what was deliberately saved |
| raw `*.jsonl` on disk | everything | only by hand-writing a parser |

So the actual procedure today is: know the on-disk layout, find the right
`.jsonl` under
`~/.agentmux/channels/<channel>/identities/<bundle>/claude/projects/<slug>/`,
and write a one-off script. An agent that doesn't already know that layout
cannot do it at all.

### 1.1 Three failures this caused in a single session (2026-09-17)

Not hypothetical — this is the session that prompted the spec:

1. **An agent asserted something false about its own actions.** Asked "why did
   you jekt Agent4?", the agent audited the only thing it could cheaply
   audit — the *current* session — found no `SendMessage` calls, and answered
   "I didn't." It had in fact sent two, in a prior session whose history had
   been lost. The audit was sound; the *scope* was wrong, and nothing made the
   wider scope reachable.
2. **A second agent made the identical error.** Agent4's denial was "I have no
   record of it **in this session**." Two agents with reset sessions can both
   truthfully deny messages they genuinely sent — which is how a routine
   session reset starts looking like an impersonation incident, and did.
3. **Neither could be settled by evidence.** Confirming #2 required Agent4's
   *older* sessions; `GetAgentTranscript` returns only the live tail, and no
   archived transcript for that agent was locatable. **Phase 1 does not fix
   this one** — see §5.

Failures 1 and 2 share a root cause worth naming precisely: **an agent's
memory of what it did is bounded by its current context window, while its
actual actions are not.** Every long-lived agent is therefore one compaction
or session reset away from confidently misreporting its own behaviour. That is
a correctness problem, not a convenience one.

---

## 2. What already exists (don't rebuild it)

`agentmux-srv/src/backend/history/` is a working discovery + parse layer:

- **`HistoryAdapter` trait** (`adapter.rs`) — one impl per CLI provider;
  `ClaudeHistoryAdapter` today, with `provider()` documenting
  `codex`/`gemini`/`kimi`/`openclaw`/`pi` as the intended shape.
- **`SessionIndex`** (`index.rs`) — `refresh()`, `list()`,
  `list_for_identity()`, `get_meta()`, `get_full()`, keyed by identity bundle
  so "this agent's sessions" is O(that identity's sessions), not a full scan.
- **`HistoryService::sessions_for_agent(store, agent_id, …)`** (`mod.rs`) —
  already resolves `agent_id` → linked identities → sessions, via
  `db_agent_identity_links`. **This is the exact primitive this spec needs**,
  and it is already in production use by
  `bundle.export_for_agent_with_history`.
- **`HistorySession { meta, messages: Vec<HistoryMessage> }`**, where
  `HistoryMessage` is `{ role, content, timestamp, tool_uses:
  Vec<ToolUseSummary{ name, argument_summary }> }`.

The `tool_uses` field matters: it means "did I call `SendMessage`, and with
what arguments" is answerable **structurally**, not by regexing prose. That is
precisely failure #1.

**Nothing new needs to be parsed, discovered, or indexed. This spec adds a
search verb and an App API tool.**

---

## 3. Design

### 3.1 `SessionIndex::search`

```rust
pub struct HistorySearchHit {
    pub session_id: String,
    pub file_path: String,
    pub timestamp: i64,
    pub role: String,          // "user" | "assistant"
    pub snippet: String,       // matched text, bounded
    pub tool_name: Option<String>, // Some(..) when the hit is a tool call
}
```

Matching runs over each candidate session's `HistoryMessage`s:

- **`content`** — case-insensitive substring by default.
- **`tool_uses[].name`** and **`.argument_summary`** — so `SendMessage` finds
  the call, not just prose mentioning it.

A hit yields a bounded `snippet` (the matched region plus context, hard-capped)
rather than the whole message — a single message can be hundreds of KB, and
this tool must never be the reason a caller blows its context window. That is
not a theoretical concern: `GetAgentTranscript` returned **112 KB across 129
lines** in the session that prompted this spec and had to be spilled to a file.

### 3.2 Bounding the scan — the part that decides whether this is usable

Full-parsing every session of a long-lived agent is not viable: the session
behind §1.1 was **14.7 MB / 6682 records**, and it was one of several.

Three bounds, all cheap because they use `SessionMeta` (already indexed, no
file read):

1. **Candidate selection** — `sessions_for_agent(…, sort_by: "modified_at",
   sort_dir: "desc")`, then take at most `max_sessions` (default 20, cap 100).
   Most questions are about recent behaviour.
2. **`since` / `until`** — filter on `SessionMeta.modified_at` **before**
   opening anything. A caller asking "did I message Agent4 in September" reads
   no September-irrelevant file at all.
3. **`limit`** — stop at `limit` hits (default 50, cap 200), and report
   `truncated: true` plus `sessions_scanned` so the caller knows the answer is
   partial rather than negative.

**`truncated` is load-bearing, not cosmetic.** A search that silently stops
early and returns nothing reproduces exactly failure #1 — an agent concluding
"I didn't do that" from an incomplete audit. An absent result must be
distinguishable from an exhausted budget, so the response always states which
it was.

### 3.3 App API tool

```
SearchHistory {
  query: string,              // required
  agent?: string,             // defaults to the calling agent (§5)
  since?: string,             // ISO 8601 or unix seconds
  until?: string,
  role?: "user" | "assistant",
  tool?: string,              // match tool_uses[].name, e.g. "SendMessage"
  max_sessions?: integer,     // default 20, cap 100
  limit?: integer,            // default 50, cap 200
}
```

Returns `{ hits: [HistorySearchHit], sessions_scanned, total_sessions,
truncated }`.

Transport mirrors `GetAgentTranscript` exactly (`agentmux-mcp` → srv HTTP with
`X-AuthKey`, per `ARCHITECTURE_NETWORK_CREDENTIAL_MAP_2026_09_06.md`'s route
gates). New route: `GET /agentmux/reactive/history/search`.

`tool` deserves its own parameter rather than relying on `query`: "find where I
called `SendMessage`" is a different question from "find where I wrote the word
SendMessage", and the structural answer is the one that settles an audit.

---

## 4. Why this is worth building beyond convenience

The failures in §1.1 are trust failures. An agent that cannot check its own
history will, when asked about its past, answer from context — and its context
is a lossy, silently-truncated view of its own actions. It will do so
confidently, because nothing signals the boundary.

This is sharpest for exactly the questions that matter most: *did I send that
message*, *did I already make this change*, *did I promise that*. All three
are audit questions, all three are answerable from disk, and today none of
them is answerable by the agent being asked.

---

## 5. Scope: own history only (Phase 1)

**The `SearchHistory` tool exposes no `agent` parameter — it always sends the
caller's own id.** Cross-agent search is deliberately excluded, and the reason
is not effort:

Reading another agent's conversation is already a governed act. `muxspect`'s
cross-tier visibility protocol and the `transcript_request` tier rules
(live since PR #2764) exist precisely to decide whether one agent may see
another's content — `conversation_visibility` of `private` / `trusted_peers` /
`ask`, with `ESCALATE=required` never relaxed for an `ask`-mode responder even
for a cryptographically verified requester. A search verb that read other
agents' transcripts directly would be a **second, ungoverned disclosure path
around that machinery** — worse than the gap it closes, because it would look
like an ordinary read tool.

Phase 2 (separate spec) should route cross-agent search *through*
`transcript_request` rather than beside it. Until then §1.1's failure #3 —
checking another agent's older sessions — stays open, and this spec says so
rather than implying otherwise.

**What is NOT true, and must not be written down as if it were: that this is
*enforced*.** The HTTP route takes an `agent` parameter (it has to — the
server must know whose sessions to resolve), and the route is gated only by
the instance-wide `auth_key` that every locally-spawned agent shares. The
server therefore cannot tell which agent is calling, so a local process
holding that key can pass any name. That is not a regression — the same is
already true of `/reactive/transcript` — but it means "own history only" is a
**client-side convention** implemented by the tool exposing no `agent`
parameter, not a boundary the server upholds. Real enforcement needs a
verifiable per-agent identity on local routes, which does not exist today;
`host_reg_secret` is the codebase's existing acknowledgement that `X-AuthKey`
alone cannot distinguish callers that share it. Anyone extending this should
fix that rather than assume it was already handled.

Also out of scope:

- **Indexing / full-text engine.** Linear scan over a bounded candidate set is
  adequate at these sizes and adds no new persistent state to keep correct. If
  profiling later says otherwise, the `SessionIndex` boundary is the right
  place to put an index behind.
- **Non-Claude providers.** Search works through the `HistoryAdapter` trait, so
  every provider with an adapter is covered automatically; only
  `ClaudeHistoryAdapter` exists today, and that is a pre-existing gap this
  spec neither widens nor fixes.
- **Regex.** Substring + a structural `tool` filter covers the audit cases.
  Regex over untrusted input on the srv thread is a denial-of-service surface
  (catastrophic backtracking) for no demonstrated need.

---

## 6. Key files

- `agentmux-srv/src/backend/history/index.rs` — `SessionIndex::search`
- `agentmux-srv/src/backend/history/mod.rs` — `HistoryService::search_for_agent`
- `agentmux-srv/src/server/reactive.rs` (or a sibling handler module) — the HTTP route
- `agentmux-mcp/src/tool_schemas.rs` — `SEARCH_HISTORY_TOOL`
- `agentmux-mcp/src/main.rs` — dispatch arm, mirroring `GetAgentTranscript`
- `docs/specs/ARCHITECTURE_NETWORK_CREDENTIAL_MAP_2026_09_06.md` — new route row, per that doc's own same-change rule

## 7. Testing

- A hit in message `content`; a hit in `tool_uses[].name`; a hit in
  `argument_summary` — the last is what answers "who did I message".
- `since`/`until` exclude sessions **without reading them** (assert via an
  adapter that records which files were opened).
- `limit` reached → `truncated: true`; budget exhausted is distinguishable from
  genuinely no matches (§3.2).
- Snippet is bounded for a pathologically large message.
- `agent` naming another agent is rejected in Phase 1, with an error that
  points at the visibility protocol rather than a bare "unsupported".
