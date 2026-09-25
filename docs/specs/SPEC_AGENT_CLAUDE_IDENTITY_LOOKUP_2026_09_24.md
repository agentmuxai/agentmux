# SPEC: an agent's own current Claude identity, in one read — no tool call, no guessing

**Date:** 2026-09-24
**Status:** proposed
**Trigger:** Repo owner, live: *"lets write a spec for an agent to quickly get
the latest claude identity"*, then, mid-turn: *"we want to know the claude
account the user is logged in as, directly from the core claude, not
guessing or using a tool call."*
**Scope:** An agent answering "which Claude account am I authenticated as,
right now, and where is my current session's transcript?" for **itself**,
cheaply and correctly. Looking up *another* agent's identity is a related
but secondary problem (§5, P3) — it cannot be solved by env vars alone and
is out of this spec's primary scope.

---

## 1. Why this is mostly already solved

Every piece exists on `main` today. Verified live, against this session's
own running agent (`agentx-0623n`, identity `60a8fde6…`), not inferred:

| Piece | State |
|---|---|
| The account's login email, recorded by Claude itself | **Yes** — the CLI writes `oauthAccount.emailAddress` to `.claude.json` inside its own config dir on every login (confirmed live: `poppercornell@gmail.com`) |
| A pointer to that config dir, in every Claude Code agent's own environment | **Yes** — `CLAUDE_CONFIG_DIR`, set at spawn for every agent (confirmed live) |
| The current session id, in-process | **Yes** — `CLAUDE_CODE_SESSION_ID` (confirmed live, matches `db_agents.session_id` for this agent exactly) |
| The transcript path for that session | **Yes** — `<CLAUDE_CONFIG_DIR>/projects/<slug>/<session_id>.jsonl`, where `<slug>` is `claude_layout::project_dir_name(cwd)` (`agentmux-srv/src/backend/claude_layout.rs:35`, byte-for-byte CLI-compatible since #3690) |
| The backend already reads this exact file for the same reason | **Yes** — `agentmux-srv/src/identity/account_email.rs::email_from_oauth_dir` reads `<dir>/.claude.json` → `oauthAccount.emailAddress` server-side, to backfill `account.context.email` (`SPEC_ACCOUNT_EMAIL_IN_ARMORY_2026_09_23.md` §4) |
| **That email reaching an agent that asks for it** | **No** — this is the entire gap |

The fix is not "build a lookup." It's "write down the four-line recipe an
agent can run itself, with no tool call," plus one narrow MCP-schema
addition for the case that genuinely needs a tool call (looking up
*another* agent).

---

## 2. The problem, evidenced

This session burned real effort rediscovering all of the above the hard
way, because nothing states it:

1. Asked "your core anthropic is poppercornell@gmail.com, right?" — the
   agent (in this same conversation) reached for `IdentityAccounts`, which
   returned an account named `"claude-oauth"` with no email at all
   (`masked_tail` blank; the MCP tool's documented return shape is
   `account_id, provider, name, kind, status, masked_tail, updated_at` —
   `context`/`email` is not in it, even though the backend account object
   carries `context.email` once §1's backfill has run).
2. Followed up with `IdentityValidate`, which failed outright
   (`keychain: keychain read failed: No matching entry found in secure
   storage`) — informative about the credential, useless for the email
   question, and cost a full round trip to learn that.
3. Landed on circumstantial evidence instead — a different email
   (`asafebgi@gmail.com`) quoted from a day-old session transcript — which
   was real, but was the *previous* identity binding, not the current one
   (this workspace's Claude identity has churned at least 10 times since
   August; `agentmux#3667`, filed 2026-09-24, is the open bug for the churn
   itself). Presenting that as an answer would have been wrong.
4. Only got the right, verifiable answer by reading `.claude.json` directly
   off disk via `CLAUDE_CONFIG_DIR` — a zero-tool-call, one-file read that
   took longer than it should have only because nothing documents that this
   is the move.

Separately, `docs/retro/retro-cross-agent-conversation-lookup-2026-09-20.md`
(this same agent, four days earlier — not previously in this repo; added by
this PR alongside this spec) hit the mirror-image problem for *another*
agent's identity: `GetAgentTranscript` reads a stale live buffer,
`SearchHistory` is deliberately self-scoped, and the only working method was
reading `objects.db`'s `db_agents` table by hand with a hand-rolled SQLite
connection (no `sqlite3` on `PATH`, no MCP tool for it). That recommendation
is still open; §5 P3 below is the same ask, scoped down to what this spec
can responsibly commit to.

---

## 3. Design

### 3.1 Self-lookup — zero tool call (the primary ask)

Four reads, no RPC, no guessing:

```bash
# 1. Login email, straight from Claude's own state
EMAIL=$(python3 -c "import json,os;print(json.load(open(os.path.join(os.environ['CLAUDE_CONFIG_DIR'],'.claude.json'))).get('oauthAccount',{}).get('emailAddress',''))")

# 2. Current session id — already in-process, not stored anywhere else first
SESSION_ID="$CLAUDE_CODE_SESSION_ID"

# 3. The project-dir slug the CLI files this cwd's sessions under
#    (same algorithm as agentmux-srv's claude_layout::project_dir_name —
#    every non-ASCII-alphanumeric UTF-16 code unit becomes '-', 200-char cap
#    then a hash; see agentmux-srv/src/backend/claude_layout.rs:35)

# 4. The live transcript
TRANSCRIPT="$CLAUDE_CONFIG_DIR/projects/<slug>/$SESSION_ID.jsonl"
```

None of this requires `IdentityAccounts`, `WhoAmI`, or any App API tool —
`CLAUDE_CONFIG_DIR` and `CLAUDE_CODE_SESSION_ID` are both already exported
into every Claude Code agent's process at spawn (confirmed live this
session; no code change needed to produce them).

**Deliverable:** this recipe does not live in a tool — it lives in
`AGENTMUX_MEMORY.md`'s Operator Config "Environment & Gotchas" entry (the
one every agent already gets at launch, see `.claude/AGENTMUX_MEMORY.md`),
as a new bullet: *"Your own current Claude login email and session
transcript are a local read, not a tool call — `$CLAUDE_CONFIG_DIR/.claude.json`'s
`oauthAccount.emailAddress`, and `$CLAUDE_CODE_SESSION_ID` for the current
session id."* Zero code to ship; it just has to stop being tribal knowledge
one session rediscovers at a time.

Non-Claude providers: `email_from_oauth_dir` (§1) already documents that
only Claude/`claude-code` write this today; an agent running under another
provider has no equivalent local file and still needs the MCP path (§3.2).

### 3.2 Cross-agent lookup — necessarily a tool call

Looking up a *different* agent's current identity can't be a local file
read (its `CLAUDE_CONFIG_DIR` isn't in this process's environment). Two
independent, small fixes cover it:

1. **Expose the email `IdentityAccounts` already computes.** Add
   `context` (or a narrower `email: Option<String>`) to the MCP tool's
   returned shape — the backend already puts it there
   (`agent_handlers/identity.rs:174`, `account.context["email"]`); the MCP
   layer (`agentmux-mcp/src/main.rs`, `tool_schemas.rs`) just never asked
   for that field. This alone fixes the exact question this session's
   `IdentityAccounts` call failed to answer, for the calling agent's *own*
   accounts.
2. **A `GetAgentIdentity` (or `WhoAmI`-extension) App API call**, scoped to
   read-only, that given an `agent_id` resolves: its current
   `db_agents.session_id`, its bound `identity_id`/account (`db_agent_identity_links`,
   confirmed empty for this agent today — a real gap this call would also
   surface, not paper over), and the transcript path via §3.1's same
   algorithm, read from `shared/identities/<id>/claude/projects/` — the
   single global tree, not `channels/<ch>/identities/<id>/...`, which
   `ensure_history_link` (`SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md`,
   PR #2605) turns into a junction/symlink into the shared copy at
   `auth.start`/spawn — the 2026-09-20 retro's "dual tree" was this
   linking, not two independently-authoritative copies. The one real gap
   (a channel retired before that link ever ran, so its directory sat
   real-but-unlinked) was a one-time migration, already merged
   (`m0034_identity_history_backfill.rs`, PR #3460, 2026-09-22) — not
   something this spec needs to redo. `GetAgentIdentity` should still fall
   back to a channel-scoped read if `shared/` is somehow missing the file
   (mirroring `ClaudeHistoryAdapter`'s own bounded fallback scan, per that
   migration's doc comment), but that is a defensive fallback, not
   evidence of an unresolved mystery.

P3 is scoped narrower than the 2026-09-20 retro's original ask (which also
wanted live pane/block metadata) — just identity + transcript path,
because that is what both this session and that retro actually needed and
neither `GetAgentTranscript` nor `SearchHistory` provide.

---

## 4. Non-goals

- **Not fixing the identity churn itself.** Why a new identity UUID gets
  minted on nearly every respawn/rebind is `agentmux#3667`'s problem, not
  this spec's. This spec makes the *current* identity fast and correct to
  read, however often it changes.
- **Not re-explaining the `channels/` vs `shared/` identity tree.** §3.2
  already resolves this: `shared/` is authoritative, `channels/.../identities/`
  is a link into it, and the one real historical gap has its own merged fix
  (PR #3460). This spec's P3 just has to read the right (shared) tree and
  fall back defensively, not investigate further.
- **Not a general "find any session by content" search** — that's
  `SearchHistory`'s job (`SPEC_AGENT_HISTORY_SEARCH_2026_09_17.md`,
  actively being hardened this week in #3693/#3711).

---

## 5. Phases

| Phase | Scope | Cost |
|---|---|---|
| **P1** | Document §3.1's recipe in `AGENTMUX_MEMORY.md`'s Operator Config env/gotchas entry | Docs only, no code |
| **P2** | Add `context`/`email` to `IdentityAccounts`'s MCP return shape (`agentmux-mcp/src/tool_schemas.rs`, `main.rs`) | Small, additive, no schema break — new optional field |
| **P3** | `GetAgentIdentity` App API call: agent_id → session_id, identity/account, transcript path (checked in both identity roots) | New read-only RPC + MCP tool |

P1 alone would have answered this session's actual question in one read
instead of two failed tool calls plus a misleading circumstantial guess.

---

## 6. Verification

- **P1**: no code, but the recipe itself is checked into this doc with the
  exact commands used live this session (§3.1) — a future agent can run
  them verbatim.
- **P2**: unit test on the MCP handler asserting the returned JSON for an
  account with a non-empty `context.email` includes it; existing
  `agent_handlers::identity` tests already cover the backfill writing
  `context.email` (`identity/account_email.rs` tests) — P2 only has to
  assert the MCP layer stops dropping a field that's already there.
- **P3**: reads `shared/identities/<id>/claude/projects/` and returns its
  file; a fixture where `shared/` is missing the session (simulating a
  channel from before `ensure_history_link` first ran, or one #3460's
  backfill somehow missed) confirms the channel-scoped fallback fires
  instead of returning nothing.

---

## 7. Open questions

- Should P2's email be masked by default (matching `masked_tail`'s existing
  privacy posture for other fields) with an explicit opt-in to see it in
  full, or is an OAuth login email not sensitive enough to warrant that
  given it is already unmasked in the Armory UI (`SPEC_ACCOUNT_EMAIL_IN_ARMORY_2026_09_23.md`)?
- None outstanding on the identity-tree question — resolved while writing
  this spec (§3.2): `shared/` is authoritative, `channels/.../identities/`
  links into it, and PR #3460 already backfilled the pre-fix orphans. Kept
  here as a record that the 2026-09-20 retro's open question has since been
  answered, not to re-open it.
