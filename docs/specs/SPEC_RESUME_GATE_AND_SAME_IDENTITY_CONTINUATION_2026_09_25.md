# SPEC: one resume gate, and native continuation across logins of the same identity

**Status:** active — Phase 1 (the chain-head resume gate) shipped in #3833; Phase 2 (identity key on segments, no resume across identities) is this change; Phases 3–4 not started. See §7.
**Date:** 2026-09-25
**Author:** AgentA (agent, `~/.agentmux/agents/agenta-07017`), at operator request
**Related:** `SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md` (§4.1 segments, §4.2 the rung
ladder, R2 "Relocate", §4.4 packet), `REPORT_AGENT_HISTORY_LOST_ON_NEW_BUILD_2026_09_25.md`
(recommendations 2 and 3), `SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md` (the lease).
**Verified against:** `origin/main` `a86ce41d2`, Claude Code CLI 2.1.280.

---

## 0. Summary

Every local build forces a new login, and every login gets a new account id and a new, empty
Claude config dir. So a rebuild can never resume the agent's conversation natively. It falls
back to a fresh session plus AgentMux's continuation packet (fixed in #3817), which is a
summary, not the conversation.

The obvious fix, reusing the history of an earlier login of the same person, is unsafe on its
own. The only thing that stops AgentMux resuming the *wrong* conversation today is that the
folders differ. Three of the four paths that pick a session to resume never check whether that
session is still where the agent's conversation is (§2.3). Once history is shared across
logins, a pane holding a stale session id would silently resume an old branch of a
conversation that has since moved on elsewhere.

This spec therefore does two things, in order:

1. **One resume gate.** Every path that is about to pass `--resume <sid>` asks the agent's
   segment chain first. Only the chain head may be resumed natively. A stale id is redirected
   to the head, or refused (the spawn then goes fresh + packet).
2. **Same-identity continuation.** The identity is the real Anthropic account
   (`accountUuid` + `organizationUuid`), **not the email**. When the chain head lives under
   another login of the same identity, AgentMux copies that one session into the current
   config dir and resumes it with `--fork-session`. The original is never written to, and the
   copy is removed once the fork has its own id. Across *different* identities nothing
   changes: fresh + packet.

Credentials are not shared or moved. The per-channel login policy
(`inject.rs` `resolve_account_for_spawn`) is untouched.

## 1. The hazard, stated precisely

A conversation "continued elsewhere" and then "picked up unknowingly by id" means this: pane P
holds session S1. The agent's conversation then continues somewhere else as S2. That can be
another channel or build on this host, another pane after a takeover, or a fork. Later P
spawns with `--resume S1`. The provider resumes S1 happily, because the file is there. The
pane says "Resumed", and the agent carries on from a point the conversation already moved
past. Nothing errors, and nobody is told.

Related failure modes this spec must also handle:

| # | Case | Why it matters |
|---|---|---|
| H1 | Pane holds S1; the chain head is S2 (conversation moved on) | Silent resume of a stale branch |
| H2 | Two live processes on one session file | `--resume` appends to the *same* file (§2.4); interleaved lines corrupt it |
| H3 | Same email, different Anthropic account or org (e.g. a personal org and a Team org) | Cross-account resume is [unverified] (durable-memory spec §4.2): thinking signatures, server-side checks |
| H4 | Session file modified outside AgentMux (a terminal `claude --resume`) | AgentMux's record no longer matches the file |
| H5 | Relocation source missing, unreadable, truncated, or copy fails midway | Must not leave a half-copied file the CLI then resumes |
| H6 | The CLI or API rejects the resume ("No conversation found", signature or 4xx error) | Must fall through within the same launch, never loop |
| H7 | The login's identity can't be read (`.claude.json` absent or unparsable) | Must behave exactly as today |
| H8 | No chain yet (agents that last ran before #3676) | Must behave exactly as today |

## 2. How it works today (verified)

### 2.1 Logins and folders
- The Armory upsert mints a fresh account UUID whenever the caller passes none
  (`server/agent_handlers/identity.rs:224`). `auth.start` wires `CLAUDE_CONFIG_DIR` to
  `identities/<account-id>/claude` **before** the OAuth flow runs.
- Who logged in is only known **afterwards**: Claude writes `oauthAccount` (with
  `emailAddress`, `accountUuid`, `organizationUuid`, …) to `<config-dir>/.claude.json`.
  `identity/account_email.rs` already reads the email from there.
- `projects/` under a channel's account dir is junctioned to
  `shared/identities/<account-id>/claude/projects` (`link_history_if_isolated`). History is
  keyed by AgentMux's account row, not by the Anthropic identity.

### 2.2 The chain
- `backend/continuity_segments.rs`: an append-only `segments.jsonl` per agent UID in the
  global transcript store (shared by every srv on the host). Each `Start` records
  `provider_session_id`, `config_dir` (per provider since #3813), `channel`, `lease_epoch`,
  `continuity_rung`, `block_id`, `cwd`.
- `head_session(segments, current)` is the session of the newest segment that has one.

### 2.3 The four paths that choose a session to resume

| Path | Source of the id | Checks the chain? |
|---|---|---|
| Hydrate from config (reopen, restart, picker reattach) | `config.session_id` (`persistent/spawn.rs:39-44`) | **No** |
| First spawn onto rendered history | `find_continuation_session_id` (`spawn.rs:51-72`) | **No** |
| Resume preflight (the pane's "will this resume?" verdict) | `resume_preflight.rs:150`, via `app_api/session.rs` | **No** |
| Recovery after a rejected resume | `find_recovery_session_id` (`resume_retry.rs:231`) | **Yes** (`resume_retry.rs:251`, `recovery_allowed`) |

The first two converge in `spawn_process` just before `requested_sid` is read
(`spawn.rs:94`). That single point also runs the held-elsewhere refusal (`spawn.rs:132`) and
the host-wide single-live-instance lease (`spawn.rs:144`).

### 2.4 The CLI (Claude Code 2.1.280, `claude --help`)
- `-r, --resume [value]`: "Resume a conversation by session ID". It continues **the same id and
  file**. This session, `7e2f6a82…`, was resumed several times and kept one file.
- `--fork-session`: "When resuming, create a new session ID instead of reusing the original".
- `--session-id <uuid>`: "Use a specific session ID for the conversation".
- The persistent controller already adopts a captured id that differs from the attempted one
  (`PersistentInner::try_capture_session_id`, `persistent/mod.rs:637`), which a fork produces.

## 3. Invariants

- **I1. Only the chain head is resumed natively.** When a chain exists, a spawn never passes
  `--resume` with any other id.
- **I2. Never across identities.** A native resume, a relocation or a fork requires the
  resumed session's identity key to equal the spawn's. An unknown key on either side counts as
  "not equal" for relocation, and as "no information" (today's behaviour) for plain same-dir
  resume.
- **I3. Never write another login's file.** A relocation reads the source and writes only
  into the current config dir, and only through `--fork-session`.
- **I4. Never two files with one id, lasting.** A relocated copy exists only until the fork
  reports its new id, and is removed then (or at the next spawn if the process died first).
- **I5. Every fallback is visible.** Redirect, relocation, refusal and a fresh start each
  leave a log line, a segment record, and the pane's session-outcome line.
- **I6. No new failure mode.** Any error inside the gate or the relocation degrades to the
  pre-spec behaviour for that spawn, never to a refused spawn.

## 4. Design

### 4.1 Identity key
`identity_key(provider, config_dir) -> Option<String>`, for Claude only:
`hex(sha256("claude\0" + accountUuid + "\0" + organizationUuid))[..16]`, read from
`<config_dir>/.claude.json` `oauthAccount`. It's `None` when the file, either field, or the
provider support is missing (H7), and it skips the ambient `~/.claude` the same way
`email_from_oauth_dir` does. It's hashed because it goes into a log shared by every srv on the
host. The email is never used as the key (H3).

It's recorded on each segment `Start` as `identity_key: Option<String>`
(`#[serde(default, skip_serializing_if = "Option::is_none")]`, same compatibility pattern as
`lease_epoch`).

### 4.2 The gate
A pure decision in `continuity_segments.rs`, called once in `spawn_process` after the id is
hydrated and after first-spawn continuation, before `requested_sid` is read. The preflight
calls the same function so the pane's verdict and the spawn cannot disagree.

Inputs: the candidate id (what `--resume` would get), the agent's segments, the spawn's config
dir, cwd and identity key, and the poisoned id. Decision:

| Condition | Decision |
|---|---|
| No `--resume` support, no candidate, no agent UID, or the chain can't be read | **Allow** (today's behaviour; I6, H8) |
| No head in the chain (only legacy or current segments) | **Allow** (H8) |
| Head is the poisoned id | **Allow** (no better information than today) |
| Candidate == head | **Allow**, then the identity check below |
| Candidate != head, head is reachable in this config dir + cwd | **Redirect** to the head (H1) |
| Candidate != head, head is not reachable here | **Relocate**, if §4.3's preconditions hold; else **Refuse** |

Identity check on Allow or Redirect (Phase 2): if the resumed session's segment has an
identity key, the spawn has one, and they differ, then **Refuse** (H3). A same-dir mismatch
means the folder was re-logged-into as someone else.

**Refuse** clears the candidate (`inner.session_id = None`, `persist_session_id("")`). The
existing no-`--resume` path then applies: fresh + continuation packet, rung `Virtualized` when
a record exists, and the pane outcome line with `attempted_sid` = the refused id.

**Redirect** sets `inner.session_id` to the head. The held-elsewhere refusal and the lease
still run on the head, unchanged. So a head that is live in another pane or instance is
refused exactly as today (H2).

### 4.3 Same-identity relocation (the durable-memory spec's R2)
Preconditions, all required. Any failure means **Refuse** (fresh + packet):
1. The head segment's provider is Claude, as is the spawn's.
2. The head segment's `config_dir` is set and differs from the spawn's.
3. `identity_key(head segment) == identity_key(spawn)`, both `Some` (I2).
4. `<head config_dir>/projects/<cwd-slug>/<head>.jsonl` is a readable, non-empty file whose
   last line parses as JSON (H5, which catches a truncated file).
5. `<spawn config_dir>/projects/<cwd-slug>/<head>.jsonl` does **not** exist (never overwrite).

Action:
1. Copy to `<dest>.jsonl.agentmux-tmp`, then rename to `<dest>.jsonl`. The copy is atomic,
   so the CLI never sees a partial file (H5). Write a marker `<dest>.jsonl.agentmux-relocated`
   holding the source path and size.
2. Spawn with `--resume <head> --fork-session`. The original stays untouched (I3), and the new
   id is captured by the existing adoption path.
3. When the new id is captured, record it on the segment (`session` event), delete the copy
   and its marker (I4), and emit outcome `resumed` with `actual_sid` = the new id.
4. At the next spawn, delete any copy that still has its marker (the process died before
   step 3).
5. If the CLI rejects the fork (H6), the existing stderr poison + retry path runs with the
   copy's id poisoned. The retry is fresh + packet, and the copy is cleaned up by step 4.

The segment records `forked_from: Option<String>` (the head id). The rung stays `Native`; no
enum change, so older builds can still fold the log.

### 4.4 Fork when the file changed outside AgentMux (H4)
Each segment's `End` records `provider_bytes_end`, the size of the provider session file when
the process went away. At the gate, if the head's file is now larger than that and no later
segment accounts for it, the conversation continued outside AgentMux. Resume with
`--fork-session`, so the external branch is never appended to by a second writer, and emit the
outcome line with `continued_externally: true` so the pane can say so.

### 4.5 The ladder, end to end
1. **Native.** Allow or Redirect, same dir, same identity.
2. **Native fork.** Relocate (§4.3) or externally modified (§4.4).
3. **Virtualized.** Refuse → fresh + continuation packet (#3817 makes the packet read the
   agent's global record).
4. **Fresh.** No record anywhere; the pane says it starts without prior context.

A CLI rejection at 1 or 2 falls to 3 within the same launch (existing poison + retry). It
never loops, because the poisoned id is excluded from the gate's candidates.

## 5. Case matrix

| Case | Today | After this spec |
|---|---|---|
| Rebuild, re-login same identity, pane holds the head | Fresh + packet (new empty dir) | **Native fork** via relocation |
| Rebuild, re-login **different** identity | Fresh + packet | Fresh + packet (I2) |
| Same email, different org | Fresh + packet (dirs differ) | Fresh + packet (key differs, H3) |
| Pane holds S1, head is S2, both in this dir (H1) | **Resumes S1 silently** | Redirect to S2 |
| Pane holds S1, head S2 is under another login of the same identity | Resumes S1 silently if reachable | Relocate S2 + fork |
| Head is live in another pane or instance (H2) | Refused (held elsewhere / lease) | Unchanged |
| Head file grew outside AgentMux (H4) | Resumed and appended in place | Fork, disclosed |
| Relocation source truncated or unreadable (H5) | n/a | Fresh + packet |
| CLI rejects the resume or fork (H6) | Poison → recovery or fresh | Unchanged; the relocated copy is cleaned up |
| `.claude.json` missing (H7) | n/a | Same-dir: today's behaviour; relocation: refused |
| No chain (H8) | Today's behaviour | Unchanged |

## 6. Recording and disclosure
- Segment `Start` gains `identity_key`, `forked_from` and `gate` (one of `allow`, `redirect`,
  `relocate`, `refuse`, plus a short reason). All are optional, so older builds skip them.
- One `tracing::info!` per non-`allow` decision, target `continuity`, with block, candidate,
  head, decision and reason.
- The pane's session-outcome line keeps its current shape. `attempted_sid` is the pane's
  candidate and `actual_sid` is what really ran.

## 7. Phases (one PR each)

1. **Resume gate, chain-head only.** The decision function; the spawn and preflight call
   sites; Redirect and Refuse. Tests: H1 redirect, unreachable head → refuse, legacy/no chain,
   poisoned head, no UID, preflight agrees with spawn.
2. **Identity key.** `identity_key()`, recorded on segments, and the identity check on
   Allow/Redirect. Tests: a key from a fixture `.claude.json`, missing fields → `None`, the
   ambient dir skipped, a same-dir different key → refuse.
3. **Same-identity relocation + fork.** §4.3 in full, including cleanup and marker recovery.
   Includes a live check of `--resume <id> --fork-session` with the stream-json flags the
   persistent controller uses. Tests: each precondition failing → refuse; atomic copy; never
   overwrite; cleanup on capture and on the next spawn.
4. **External modification.** `provider_bytes_end` on `End`; the fork + disclosure of §4.4.

Each phase updates this Status line.

## 8. Non-goals
- Sharing or moving credentials between logins, or relaxing per-channel login isolation.
- Resuming across different identities (H3), which stays [unverified].
- Providers other than Claude, for relocation. The chain-head gate (Phase 1) is
  provider-neutral wherever a reachability check exists.
- Deduplicating old account rows; only history continuity is in scope.
