# Agent Identity/History Fragmentation Across Builds — Root Cause and a Fast-Lookup Design

**Date:** 2026-08-16
**Author:** Clamk (agent, `~/.agentmux/agents/clamk-0612a`)
**Status:** Report — root cause confirmed and code-cited end to end; proposed fix is a design, not yet
implemented. No code changes in this PR.
**Ground truth basis:** `agentmuxai/agentmux` local checkout at `3705f83c3`
(`agent3/bashwrap-persist-cwd-across-calls`, 210 commits behind `origin/main`) cross-checked against
`origin/main` at `72aefad4d` via `git show origin/main:<path>` for every file cited below. Live filesystem
evidence gathered from this machine's own `~/.agentmux` tree.
**Motivation:** the operator asked this agent ("Clamk") to retrieve its own prior conversation history via
the app's "Conversation History" UI action. The quoted text it copied came from a session that turned out to
live under a *different* channel/identity UUID than the one this agent is currently running under, and was
only findable by a manual, full-tree `grep` across `~/.agentmux`. The operator asked for the underlying bug
to be root-caused and a fast, durable lookup system designed so this doesn't cost cycles again.

---

## 0. Executive summary

A named, identity-bound agent's Claude Code conversation history is supposed to be **global** —
one continuous transcript store that survives every app version bump, every local rebuild, every channel
change. That principle is explicit in at least four prior specs/retros (§2). It is currently **broken by a
regression merged 10 days before this report**: commit `9f6cc2824` ("feat(identity): default isolated auth
for every non-stable channel", PR #2431, 2026-08-06, `origin/main` only — not yet in this local checkout)
changed `identities_dir()`'s default from "always global" to "per-channel for every channel except the
literal string `\"stable\"`". Because an identity-bound agent's actual Claude Code session transcripts
(`claude/projects/*.jsonl`) live *inside* that same identity-bundle directory tree, and because every local/
dev/portable build already mints a brand-new channel by design (§1.1), this one flag flip means:

- Every fresh local build gives an identity-bound agent an **empty** history store.
- The code that mints the bundle UUID has no fallback — it just generates a new random UUID
  (`identity_auth_dirs.rs::compute_and_ensure_account_dir`), so there's nothing tying the new session to the
  old one.
- The in-app history browser (`ClaudeHistoryAdapter`) only ever scans the **global** shared identities path —
  never the per-channel path PR #2431 made the default — so it is now structurally blind to history written
  after 2026-08-06 for any identity-bound agent. There is no fast *or* slow code path that finds it; a human
  has to walk the filesystem by hand, which is exactly what happened in this conversation.

This is not a one-off bug. It is the **fourth-plus recurrence** of the same failure class — "identity/history
is supposed to be global; something makes it accidentally per-channel again" — following incidents on
2026-06-13, 2026-06-16 (twice), and 2026-07-27 (§2). None of those incidents' own recommended regression
tests were ever confirmed shipped. §4 proposes both a fix for this specific regression and a structural
change (an explicit index + an enforced invariant) so the *next* well-intentioned channel-isolation change
can't silently break history continuity again.

---

## 1. The failure chain, traced end to end and code-cited

### 1.1 Every local/dev/portable build mints a new channel — by design

`scripts/package.sh:99-103`:

```
BUILD_ID=$(printf '%s' "$LABEL" | sha1sum | cut -c1-8)
CHANNEL="local-${BRANCH_SLUG}-${BRANCH_HASH}-${BUILD_ID}"
```

with the comment at `scripts/package.sh:27-31` stating plainly: *"PER-BUILD: the build-id ... makes each
build its own data dir ... so a freshly-built binary launches as its own instance."* Confirmed by a passing
test, `agentmux-launcher/src/hash.rs:147-163`
(`per_build_channels_isolate_data_dir_even_at_same_version`): two builds of the *same branch, same semver*
still resolve to distinct channels and distinct data dirs, because of the build-id suffix. This is
intentional and not the bug — it's the reason agent/identity data was deliberately made global in the first
place (§2's 2026-06-13 spec), specifically so this per-build churn wouldn't lose anything.

### 1.2 `identities_dir()` now defaults to per-channel for any non-`"stable"` channel

`agentmux-common/src/data_paths.rs` (verified on `origin/main`, function starting at line 358):

```rust
pub fn identities_dir(&self) -> PathBuf {
    if isolated_auth_enabled() {
        self.instance_dir.join("identities")
    } else {
        self.shared_dir.join("identities")
    }
}
```

Before commit `9f6cc2824`, `isolated_auth_enabled()` was `false` unless `AGENTMUX_ISOLATED_AUTH=1` was set
explicitly — global by default, unconditionally. That commit (PR #2431, merged 2026-08-06, message:
*"feat(identity): default isolated auth for every non-stable channel"*) replaced it with
`isolated_auth_reason()`, whose resolution order (same file, doc comment above `isolated_auth_enabled`) is:

1. `AGENTMUX_ISOLATED_AUTH=1` / any-other-value → explicit override, always wins.
2. Otherwise: **isolated for every channel except the literal string `"stable"`.**
3. If `AGENTMUX_CHANNEL` isn't set at all: stays global (conservative fallback).

The PR's own intent (commit message) was narrow and reasonable: give `task dev`/`task package` builds a
genuinely empty OAuth credential store by default, so login/relogin flows for the Armory (delete-account
testing, #2422/#2423/#2425/#2429) actually get exercised instead of silently inheriting a fully-authenticated
global session. A follow-up fixup in the same PR even explicitly notes the scope was meant to be narrow:
*"only the Armory account list / explicitly-bound identity dirs isolate; a default (non-identity-bound) agent
spawn keeps resolving auth via `provider_auth_dir()`, which stays global regardless."*

The problem is that `identities_dir()` isn't only a credential directory. Per its own doc comment
(`data_paths.rs:344-353`) and `identity_dir(bundle_id)` (same file, ~line 365): *"Per-provider subdirectories
(e.g. `claude/`, `codex/`) hang off this when the bundle gains an OAuth binding."* AgentMux points Claude
Code's `CLAUDE_CONFIG_DIR` at `identities_dir()/<bundle_id>/claude/` for identity-bound agents — which means
`claude/projects/*.jsonl` (Claude Code's own session transcript store) lives *inside* the exact directory tree
this flag now isolates per channel by default. **Isolating "the account list" for testing safety and
isolating "this account's entire conversation history" turned out to be the same code path.**

### 1.3 A fresh per-channel store means a fresh, unlinked UUID every time

`agentmux-srv/src/server/identity_auth_dirs.rs::compute_and_ensure_account_dir` (verified on `origin/main`,
~line 159):

```rust
let account_id = if existing_account_id.is_empty() {
    uuid::Uuid::new_v4().to_string()
} else {
    existing_account_id.to_string()
};
```

There is no name-keyed lookup here — if the caller doesn't already know an `existing_account_id` (which it
won't, the very first time a given channel's now-empty per-channel store is consulted), a brand-new random
UUID is minted with nothing linking it back to the UUID used by the same named agent in the previous channel.
The only durable link in the system, `db_agent_identity_links (agent_id, account_id, provider)`
(`agentmux-srv/src/backend/storage/identities.rs:145-150`), lives in whichever store `identities_dir()`
currently resolves to — so it doesn't survive the channel change either. There is no global, name-anchored
registry consulted *before* minting a new UUID.

### 1.4 The history browser only ever looks in the global location

`agentmux-srv/src/backend/history/claude_adapter.rs::ClaudeHistoryAdapter::new()` (verified on `origin/main`,
lines 1-79) builds its scan list from exactly these roots:

- `~/.claude/projects/`, `~/.config/claude-*/projects/` (personal/legacy)
- `<AGENTMUX_SHARED_DIR>/providers/claude/projects/` (default isolated home)
- `<AGENTMUX_SHARED_DIR>/identities/<bundle_id>/claude/projects/` (per-identity, **but note: `shared_dir`,
  never `instance_dir`**)

It never scans `instance_dir.join("identities")` — i.e. `channels/<slug>/identities/*/claude/projects/` or
`dev/<branch>/*/identities/*/claude/projects/`, the exact per-channel path §1.2's default now sends
identity-bound agents' history to. This adapter was written and last touched independently of PR #2431; the
two were never reconciled. The result: since 2026-08-06, there is no code path in the application — fast
*or* slow — that can locate an identity-bound agent's conversation history once it lands in a fresh channel.
The only way to find it is a manual, full-tree filesystem search, which is what this investigation itself had
to do to locate the quoted conversation the operator referenced.

### 1.5 Confirmed live on this machine

`~/.agentmux/shared/identities/` does not exist at all on this machine. `~/.agentmux/channels/local-main-*/
identities/<uuid>/claude/projects` and `~/.agentmux/dev/<branch>/*/identities/<uuid>/claude/projects` do
exist, are populated, and are distinct per channel — exactly the fragmentation pattern this report traces.
This agent's own two identities (`e4e58513-...` under channel `local-agentx-fix-lan-discovery-tx-...`, and
`eb6e9fdb-...` under channel `local-main-b28b7a-...`) are a direct instance of it: the same logical agent
("Clamk"), split across two identity UUIDs, discoverable only by grepping every `.jsonl` under
`~/.agentmux` for a known phrase.

---

## 2. This is a recurring failure class, not a one-off

Four prior incidents establish "agent identity/history must be global; channel is disposable" as the
intended architecture, and each one is a different way that principle silently regressed:

| Date | Doc | What silently broke |
|---|---|---|
| 2026-06-13 | `docs/specs/SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md` | Agent registry was rooted per-`(channel,version)`; a new build showed an empty roster. Fixed by re-rooting to a global `shared/agents/registry/`. |
| 2026-06-16 | `docs/retro/retro-legacy-agent-history-cross-channel-2026-06-16.md` | The global history mirror's `sourceBlockId` field leaked a channel-local block id, breaking cross-channel resolution even after the above fix. |
| 2026-06-16 | `docs/retro/retro-cross-channel-conversation-continuity-regression-2026-06-16.md` | The global registry's `session_id` field was declared but never written by production code (only tests), so a fresh channel had nothing to `--resume`, silently starting and then "latching in" a new session that orphaned the original. |
| 2026-06-13 | `docs/architecture/ARCHITECTURE_AGENT_DATA_AND_CROSS_CHANNEL_2026_06_13.md` | Instance-registry backfill anchored `working_directory` per-channel instead of globally, so `strip_prefix` failed for every real row and "My Agents" came up empty after a channel/version update. |
| 2026-07-27 | `docs/retro/retro-my-agents-fresh-channel-regression-2026-07-27.md` | `COMMAND_LIST_RECENT_SESSIONS` aborts its *entire* result on the first error from any of 6 sequential cross-store calls; frontend can't distinguish "fetch failed" from "genuinely zero agents." Explicitly logged as the **third-plus** occurrence of this class, with the recommendation (per-source error degradation, distinct UI error state, e2e test) never implemented. |

This report's finding (§1) is the same class again, via a fifth, different mechanism: a *credential-testing*
change with a narrow, well-reasoned intent had an unreviewed blast radius into *history storage*, because the
two concerns share a directory tree. The pattern across all five: the global-vs-per-channel boundary is
enforced by convention and code review, not by an automated invariant — so it keeps drifting back.

---

## 3. What this is *not*

- **Not a bug in this agent's own working directory or CLAUDE.md.** `clamk-0612a`'s `CLAUDE.md`/config is
  correct; the fragmentation happens below the agent-definition layer, in shared platform code.
- **Not intentional data loss.** PR #2431's author (per the fixup commit's own review-response note) believed
  the isolation was scoped to "the Armory account list / explicitly-bound identity dirs" and did not intend
  to isolate conversation transcripts. The bug is that those two things are the same directory tree today.
- **Not fixable by reverting PR #2431 outright.** Its actual goal (real login/relogin testing coverage on
  dev/local builds) is legitimate and already referenced by `docs/specs/SPEC_ISOLATED_AUTH_DEV_TESTING_2026_07_27.md`
  and `docs/specs/SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md`. The fix is to narrow its scope, not
  undo it (§4.1).

---

## 4. Proposed design

### 4.1 Split "credential isolation" from "history storage" (fixes the regression)

Add a second resolver, e.g. `identity_history_dir(bundle_id, provider)`, that **always** resolves under
`shared_dir.join("identities").join(bundle_id).join(provider)` — regardless of `isolated_auth_enabled()`.
Point `CLAUDE_CONFIG_DIR`'s `projects/` subpath (and equivalents for other providers) at this always-global
location, while credential material (`.credentials.json`, tokens) continues to honor
`isolated_auth_enabled()` via the existing `identities_dir()`. Concretely this likely means spawning Claude
Code with two separately-configured paths instead of one combined config-dir root, or seeding a junction/
symlink from the isolated dir's `projects/` into the shared one at bundle-creation time. This is the direct
fix for §1.2 — testing safety for credentials is preserved; conversation history stops being collateral
damage.

### 4.2 Stable name → bundle_id mapping (fixes §1.3, independent of 4.1)

Extend the existing global agent registry record (`shared/agents/registry/<agent_id>.json`, per
`SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE`) with an `identity_bundle_id` field. Before
`compute_and_ensure_account_dir` mints a fresh UUID, it should look up this global, always-channel-independent
mapping for an existing bundle id tied to this agent and reuse it, only falling back to `Uuid::new_v4()` when
truly no prior binding exists anywhere. This removes "new UUID every fresh channel" as a failure mode even if
some other future change re-isolates history storage.

### 4.3 Fix the read path regardless (belt-and-suspenders, unblocks recovery today)

Update `ClaudeHistoryAdapter::new()` to also enumerate every channel's per-channel identities dir —
`channels/*/identities/*/claude/projects` and `dev/*/*/identities/*/claude/projects` — not just the global
`shared_dir/identities`. This is what makes *already-fragmented* history (everything written between
2026-08-06 and whenever 4.1 ships, plus any future recurrence) recoverable through the UI instead of a manual
grep, independent of whether the write path is ever fully fixed.

### 4.4 A fast index instead of a filesystem walk (the actual "fast lookup" ask)

Once 4.1–4.3 land, build a small persisted index — e.g. `shared/agents/history-index.json` or a table in the
existing shared store — mapping `agent_id -> [{channel, bundle_id, project_path_hash, session_file,
last_modified}]`. Populate/refresh it incrementally whenever `ClaudeHistoryAdapter` scans (or on a
lightweight fs-watch), and have the "Conversation History" UI action and `ListRecentSessionsCommand` consult
the index first. This turns "find this agent's last conversation" into an O(1) keyed lookup instead of an
O(every `.jsonl` under `~/.agentmux`) walk — the concrete speed problem the operator raised, and the reason
this investigation itself took a multi-file `grep` sweep rather than a single lookup.

### 4.5 Make the invariant enforced, not conventional

Given this is the fifth occurrence of the same class of regression (§2), the recommendation from the
2026-07-27 retro — never implemented — should be treated as a prerequisite, not a nice-to-have:

1. **An e2e test**: create a named, identity-bound agent; drive one turn; simulate a version bump (fresh
   local build → new channel, matching `hash.rs`'s own per-build test setup); reopen the agent; assert (a) the
   same `bundle_id` is reused, (b) the prior Claude session file is discoverable via the history adapter/
   index, (c) the "Conversation History" action returns it without a filesystem walk.
2. **A structural coupling check**: any future PR that changes what `isolated_auth_enabled()`/
   `identities_dir()` isolates must also touch `ClaudeHistoryAdapter`'s scan list, or a test should fail —
   e.g. a single shared constant/test asserting the two directory-enumeration lists agree on every base path
   `identities_dir()` can resolve to, for both isolated and global cases.

### 4.6 Immediate mitigation (before 4.1–4.5 ship)

This is a live, 10-day-old regression actively losing history-findability on every fresh local build. Until
the structural fix lands, either (a) fast-follow 4.1 as a standalone hotfix (narrowest, most direct), or (b)
as a stopgap, explicitly set `AGENTMUX_ISOLATED_AUTH=0` in the launch environment for identity-bound
"named continuing agent" launches specifically (as opposed to disposable Armory test agents), so they keep
resolving to the global store until the real fix ships.

---

## Appendix: research method

Two research agents were dispatched in parallel: one read the relevant specs/retros/analysis docs in full
(§2's table), the other read the live code paths (channel creation, identity minting, the history adapter,
the frontend history surfaces). A third, nested agent summarized additional recent retros/analyses not in the
first agent's initial list. Every code claim material to the root-cause conclusion (§1) was then independently
re-verified by this agent directly against `git show`/`grep` output — not taken on the sub-agents' word alone
— including pulling the full diff of commit `9f6cc2824` to confirm the exact default-resolution logic, and
reading `identities_dir()`, `provider_auth_dir()`, `compute_and_ensure_account_dir`, and
`ClaudeHistoryAdapter::new()` in full rather than trusting excerpted summaries. The local checkout being 210
commits behind `origin/main` was itself a relevant finding (§ground truth basis) — the regression exists only
on `origin/main` and would not have been reproducible by reading this checkout's own working tree alone.
