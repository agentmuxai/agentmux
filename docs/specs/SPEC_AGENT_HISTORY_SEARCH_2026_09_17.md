# SPEC: agent-facing conversation history search

**Date:** 2026-09-17
**Status:** implemented — Phase 1 shipped in PR #3321 (first released in
v0.56.4). Re-verified against `agentmux` @ `90aa773` on 2026-09-18:
`SearchHistory` in `agentmux-mcp/src/tool_schemas.rs`, `SessionIndex::search_sessions`
and `HistorySearchHit` in `agentmux-srv/src/backend/history/index.rs`. Phase 2
(cross-agent search, §5) is deliberately out of scope here and still needs its
own spec. Revised 2026-09-24: correctness fixes after a confident "no history"
for sessions that existed (§8), owner-by-token enforced server-side (§5), and
sessions attributed by the agent's own UID-keyed record, including sessions
whose transcript is gone (§9).
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

**Enforcement (revised 2026-09-24).** When this was written the route was
gated only by the instance-wide `auth_key` every local agent shares, so "own
history only" was a client-side convention. Identity M1a gave every agent
process its own token and M4c-2c made the owner the token's row. Since
2026-09-24 the server also **refuses a request without a token (403)** rather
than resolving the self-declared `agent` name (§8), so the boundary is now
upheld server-side, as strongly as the per-agent token is kept secret.

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

## 8. Correctness fixes (2026-09-24)

**Trigger.** An agent lost its conversation to an account switch and asked
SearchHistory for it. The answer was `{hits: [], sessions_scanned: 13,
total_sessions: 13, truncated: false}` — a confident "no history" — for words
certainly present in its previous session *and its current one*. Both sessions
sat under identity bundles created after srv started, and the Claude adapter
listed bundle directories once, at startup. An audit of the rest of the path
found more ways to answer confidently and wrongly.

| Defect | Fix |
|---|---|
| Bundle / channel directories listed once at srv start, so anything created later was invisible (the incident) | `ClaudeHistoryAdapter` keeps its roots and lists the directories under them on every discovery |
| `since`/`until` documented as seconds, compared against milliseconds: `until` excluded every session, `since` excluded none | converted at the route; a millisecond value (e.g. a hit's own `timestamp`) is accepted too. Applied to messages, not just sessions; a session is skipped by `until` only if it *started* after it (it used to be by last write) |
| `tool: "SendMessage"` never matched the recorded `mcp__agentmux__SendMessage` | a bare name matches the MCP-prefixed one |
| `max_sessions` cut applied before counting; an unreadable file counted as scanned; `truncated` only meant "hit limit" | `total_sessions` counts the whole window; new `sessions_skipped`, `sessions_unreadable`, `sessions_partly_read`, and **`complete`** with `incomplete_reasons` (`hit_limit`, `max_sessions`, `unreadable_sessions`, `unreadable_records`, `discovery_errors`). Only `complete: true` with no hits means "did not happen" |
| A request without a token resolved its self-declared name — on the identity store, which has no `db_agents` table, so it guessed | refused with 403 and counted (`history.search_refused_unattributed`) |
| "Index still building" was HTTP 500 | 503 with `Retry-After` |
| A search reused an index snapshot up to 30 s old, or whenever another refresh was running, and could still answer `complete: true` without a session written in that window (Codex P1) | every search refreshes first (incremental: re-list, re-parse only changed files; 51,671 files stat'd in 2.7 s on one heavy host); only the very first build answers 503 |
| Every search running its own full refresh, serialized: four at once put the fourth past the MCP's 10 s timeout (Codex P1) | a refresh that started after a request began covers it; concurrent searches share refreshes, at most two walks however many arrive (`refresh_covering_now`) |
| A directory discovery couldn't read, or a transcript it couldn't index, was skipped in silence (Codex P1) | kept with the snapshot; the answer lists them in `discovery_errors` and is incomplete (`discovery_errors`). A path that vanished isn't counted |
| `until` dropped a session by its first *dated* record, though an undated record before it may be in the window (Codex P2) | the index notes `starts_undated`; such a session isn't ruled out by its start |
| A transcript record that couldn't be parsed was skipped in silence, so the session counted as fully read (Codex P1) | the parser counts it (`skipped_records`); the session is searched but listed in `sessions_partly_read`, and the answer is incomplete (`unreadable_records`). An unterminated last line is an append in progress, not counted |
| The MCP parsed the body before the status (a non-JSON error lost its status) and reported every non-timeout transport failure as "error sending request" (#3473) | one helper, `srv_get_json`: status first, srv's own message kept, refused connection / timeout / 401 / 503 told apart |

Still open after §8: attribution by the agent's UID-keyed record rather than
account links + working directory (§9), tool arguments beyond the first key,
tool results and subagent transcripts, and a content index.

## 9. Whose sessions: the agent's own record (2026-09-24)

**Why.** Account links and working directory were the only way to say which
sessions are an agent's, and both break: an account switch leaves the old
account's sessions behind, a shared folder or account pulls in another agent's,
and a transcript deleted by the provider's cleanup is gone from the search
entirely. Measured across the 15 agents on one host: the working-directory
match found 114 sessions, **every one** of which is also in the agent's own
record; and **75 of the 190** sessions in those records have no provider
transcript left. The search couldn't see 39% of its agents' history.

**The record.** AgentMux mirrors every agent's provider output into the global
transcript store: zone `agent:<id>:current`, archived as
`agent:<id>:archive:<ms>` by "New conversation". For a user agent `<id>` is its
UID, the same in every channel and version and across account switches.
Every line the provider writes carries its `session_id`; a typed message
(echoed by AgentMux, no id) belongs to the session of the next line that has
one. `backend/history/record.rs` reads the caller's own zones — never another
id's, never a template's shared zone — incrementally (each zone remembers how
far it was read; a zone that shrank is read again), dates lines from
`output.tsidx`, and adds the sessions the segment log (#3676) names.

**The search.**
- Candidates are the sessions the record names. One whose provider transcript
  still exists is searched there (`source: "provider_transcript"`); one whose
  transcript is gone is searched in the record (`source: "agentmux_record"`),
  through the same message converter as the transcript parser
  (`claude_adapter::message_from_entry`).
- Sessions found only by working directory or account are left out when the
  agent has a record, and counted (`inferred_sessions_excluded`);
  `include_inferred: true` searches them, labelled `attribution: "inferred"`.
  An agent with no record keeps them — it has nothing better.
- A session the segment log names that neither a transcript nor the record
  holds is reported unreadable, so the answer isn't `complete`.
- New fields: `ledger_sessions`, `record_only_sessions`,
  `inferred_sessions_excluded`; per hit `source`, `attribution`.

**Cost** (release build, the 804 MB record of one long-lived agent, 28
sessions): first scan 1.2 s, later scans 62 ms (only what was appended), one
record-only session's 5,933 messages read in 157 ms. The first scan was 12 s
until `FileStore::read_at` read a range of parts in one query instead of one
query per 64 KB part. The provider's non-Claude output (no `session_id`) never
starts a session here.
