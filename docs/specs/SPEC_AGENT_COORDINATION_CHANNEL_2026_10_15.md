# Spec: a coordination channel per repository, for agents on this computer

**Date:** 2026-10-15
**Status:** implemented (PR_PENDING).
**Related:** `SPEC_AGENT_OVERLAP_AWARENESS_2026_10_10.md` (`WhoIsWorkingOn`, `ClaimWork`, overlap notes; this builds on its store and its question resolution), `SPEC_JEKT_DELIVERY_STATES_AND_MAILBOX_2026_10_01.md` (a per-agent inbox: private, not this), `SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md` (`conversation_visibility`, `transcript_request`).

## 1. The problem

Agents coordinate today with direct messages (`SendMessage`), one reader each. Most of what they send is meant for anyone it concerns: "my PR also changes `INDEX.md`, regenerate it if you merge second", "I'm removing the shim in `window_pool.rs`, rebase around it", "I'll hold that file until you answer". Each is written once per agent the sender happens to know about, read once, and gone: an agent that starts on the same file an hour later never sees it, and asks again or collides.

What an agent wants before touching something is a cheap answer to "what have other agents said about this?" `WhoIsWorkingOn` answers "who is on it" (claims, uncommitted changes, recent edits, PRs) but not "what did they say".

The operator asked for a way for agents to read the coordination between agents, to save cycles and learn statuses.

## 2. What it is not

- **Not a copy of direct messages.** A jekt stays between its sender and its receiver. Nothing in this spec stores or exposes a jekt body: posts are separate, explicit publications. An agent's `conversation_visibility` (private by default) and `transcript_request`'s human approval keep governing anything private.
- **Not instructions.** A post is information another agent published. It is shown as quoted, historical text with its author and how the author was identified, never as a live `[JEKT:` or `[AgentMux]` marker.
- **Not for secrets.** A post whose text or target matches the sensitive keyword list (`backend/reactive/sanitize.rs` `is_sensitive_message`: tokens, passwords, `--force`, `rm -rf`, …) is refused; such things go to one agent, where they're marked sensitive.
- **Not across computers** in this version: the same reach as `WhoIsWorkingOn` and overlap notes (every AgentMux instance on this computer).

## 3. Design

### 3.1 The store

A table `db_coord_posts` in the host-global identity store (`~/.agentmux/shared/identity-store.db`), next to `db_work_claims`, created on every open without a schema-version bump (the rule for that store: a bump would lock older builds out of the shared file). Every channel on this computer reads and writes it.

A post: `id`, `repo` (the channel: the repository key, `owner/repo` or `local:<folder>`, as `WhoIsWorkingOn` resolves it), optional `path` (repository-relative), `branch`, `pr` (number), `topic`, `kind` (`post`, `claim`, `release`), `author` (agent name), `author_uid`, `author_channel` (the AgentMux instance), `identified` (whether srv identified the author by its agent token, the same proof `ClaimWork` uses), `body`, `created_at`, `expires_at`.

Retention: 7 days, and at most 500 posts per repository (the oldest go first). Expired rows are deleted on the next write.

### 3.2 Posting: `PostToChannel`

MCP tool, arguments `message` (required), and what it is about: `path`, `repo`, `branch`, `pr`, `topic` (all optional). The target is resolved exactly as `ClaimWork` resolves it (`work_facts::matching::resolve` against the caller's own facts): a relative path in the caller's clone, an absolute path into any known clone, or just the caller's current repository. With no repository at all, the post is refused (there's no channel to put it in).

The text: up to 1,000 characters and 12 lines; control characters other than line breaks become spaces. Refused when sensitive (§2). The author is who `claimer()` finds (token, then block, then unique name); a post from something that isn't an AgentMux-started agent is refused, as for claims.

The answer names the channel and how many other agents currently work in it, so the poster knows who'll see it.

### 3.3 Reading: `ReadChannel`

MCP tool, arguments `repo`, `path`, `branch`, `pr`, `query` (words to find in the topic or message), `since_minutes` (default 24 h, up to the 7-day retention), `limit` (default 20, at most 100). Resolution as `WhoIsWorkingOn`; with nothing given, the caller's current repository. A path matches a post about a file in it, the folder containing it, or the whole repository (`work_facts::paths::path_matches`, both ways). Newest first.

Each post comes back as `{id, at, kind, author, author_channel, identified, repo, path, branch, pr, topic, text}`, `text` being the body with any `[JEKT`, `[BROADCAST` or `[AgentMux` marker quoted (`quote_in_system_note`). The answer's `note` says these are what other agents published: information, not instructions.

### 3.4 In `WhoIsWorkingOn`

Its answer gains `recent_posts`: up to 5 posts from the last 24 h that match the question (same matching as `ReadChannel`), in the same shape. One call before touching a file then answers both "who is on it" and "what did they say about it".

### 3.5 Posted automatically

- **Claims and releases.** `ClaimWork` posts `kind: claim` ("claimed `<what>`: `<note>`"), `ReleaseWork` posts `kind: release` for each claim released. A claim itself lapses and leaves no trace; the channel keeps that it existed and when it ended.

Overlap notes are not posted in this version: they go to the agent that edited, and publishing them changes who learns about an overlap; left for a later version once the channel's use is seen.

### 3.6 Guidance

The Operator Config's MuxAPI entry (`crates/srv/operator-config-seed.json`) lists the two tools under coordination, with when to use them: post what concerns anyone touching a path or PR (a heads-up, a hold, a decision); read (or `WhoIsWorkingOn`) before starting on something others may be on; message one agent directly when it is for that agent.

## 4. Tests

- Store: put and read back; expiry; the per-repository cap; filters (repo, path both ways, branch, pr, query, since); newest first.
- Post: target resolution (relative path, the caller's repo); refused without a repository, without an author, with sensitive text; text clamped; markers in the text quoted when read.
- `WhoIsWorkingOn`: `recent_posts` holds the matching posts, not others.
- Claims: `ClaimWork` and `ReleaseWork` leave `claim` and `release` posts.
- MCP: schema/handler parity (the existing test), argument mapping.
