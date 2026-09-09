# Retro: a session transcript appearing to "move" between channel dirs turned out to be a working junction, not divergence

**Status:** historical
**Date:** 2026-09-08
**Area:** `agentmux-common/src/data_paths.rs` (`identities_dir`, `identity_history_dir`,
`ensure_history_link`), `agentmux-srv/src/server/identity_auth_dirs.rs` (`link_history_if_isolated`),
`agentmux-srv/src/backend/history/claude_adapter.rs`
**Context:** surfaced mid-way through a host disk-space cleanup (`~/.agentmux/channels` was one
of the largest directories on a nearly-full drive) — this is not itself a disk-cleanup retro,
it's the architecture question that cleanup raised and paused on.

---

## 1. Symptom (as observed)

While auditing `~/.agentmux/channels/` for space to reclaim, I (an agent, identity uuid
`908412be-0d37-4a73-a1c5-46fcdfd5870a`) found my own live Claude Code session transcript
(`identities/908412be.../claude/projects/.../94596c0c-....jsonl`) apparently present under
**three different channel directories**, with three different "most recently modified"
timestamps:

| Channel | Newest file mtime |
|---|---|
| `local-main-b28b7a-0d9e24f4` | 2026-09-05 |
| `local-main-b28b7a-13d6e7d1` | 2026-09-06 |
| `local-main-b28b7a-7c55a408` | 2026-09-08 (today's `$AGENTMUX_CHANNEL`, matching the currently-running instance) |

Read naively, this looks like the app copies (or independently re-writes) an identity's
session history into a fresh channel directory on every relaunch, leaving stale copies behind
— which would make the older two channels' `identities/908412be.../claude/` directories
**not** safe to delete without checking for content divergence first, since a naive read
can't tell "stale duplicate" apart from "independent write that only exists here."

This paused the cleanup: 3.2 GB was on the table in `channels/`, but not at the cost of
possibly destroying irreplaceable session history for an ambiguity I couldn't resolve by
inspection alone.

## 2. What's actually happening

It's neither a copy nor an independent per-channel write. Directly verified on this machine
(same inode, size, and mtime reachable via all three channel-scoped paths and the global
path): `identities/<uuid>/claude/projects` is a **Windows directory junction** (the
Unix-symlink equivalent) pointing at one canonical location. There is exactly one physical
transcript file; the three channel directories are three different doors into the same room.

The "different mtimes" I was reading were **not** the transcript's mtime — they were the
*parent* `identities/<uuid>/claude/` directory's own mtime, driven by that channel's real,
genuinely-per-channel files (`.claude.json`, `.credentials.json` — Claude Code's own
settings/trust state and that channel's isolated OAuth credentials). Those *are* real,
non-mirrored, per-channel data. The transcript sitting one level deeper is not.

### The actual mechanism

- `DataPaths::identities_dir()` (`data_paths.rs:370`) resolves to a **real, per-channel**
  directory whenever isolated auth is enabled — the default for any non-`"stable"` channel as
  of `SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06`. This is deliberate: it's what gives
  every `local-*` build its own isolated OAuth account list, per CLAUDE.md's own documented
  design.
- `DataPaths::identity_history_dir()` (`data_paths.rs:405-411`) is a **separate** resolver
  that always points at one global, channel-independent location, regardless of isolation.
- `ensure_history_link()` (`data_paths.rs:483` on) creates the junction from the per-channel
  `identities/<uuid>/claude/projects` to that global location. It's invoked via
  `link_history_if_isolated()` (`identity_auth_dirs.rs:37-83`) both at OAuth account/bundle
  provisioning **and** at ordinary agent-spawn time — so it's re-verified on every relaunch,
  which is exactly the behavior that made three separately-launched channels each end up with
  a working link to the same file.
- `claude_adapter.rs`'s history scanner dedupes by **canonicalized** (junction-resolved) path
  specifically so a working link doesn't get double-counted as two sessions.

This is the concrete mechanism behind CLAUDE.md's line: *"agent definitions/registry/
transcripts are GLOBAL (cross-channel work #1387–#1393): a fresh per-build data dir still
shows every agent."* It shipped **2026-08-16/17** (PRs #2605, #2606, #2611, then the
`identity_store` split in #2632) — about three weeks before the `958a0b92f` HEAD I was
auditing from. It is stable, already-shipped behavior, not something that changed in the
window I was looking at.

## 3. The part that's still a real, open gap

The junction mechanism answers "is the transcript safe" — it does not answer "why is
`channels/` still 3.2 GB of mostly-dead directories." Two things confirmed separately:

- **`docs/specs/SPEC_LOCAL_CHANNEL_PRUNER_2026_06_25.md`** (Status: **Draft**) proposes exactly
  the pipe-liveness-based pruner this problem needs. **No `pruner.rs` exists anywhere in the
  tree** — it was never built. CLAUDE.md's own text already says as much: *"pruning them
  safely needs the launcher's pipe-liveness signal, so it's a tracked follow-up... Clean up
  `~/.agentmux/channels/local-*` with no running instance manually meanwhile."*
- **The junction mechanism is explicitly best-effort**, and the code says so itself —
  `claude_adapter.rs`'s module doc (lines 8-17) flags that history written *before* the
  2026-08-16 fix landed, or for a bundle whose link creation failed (logged via
  `tracing::warn!` under target `"identity"` in `identity_auth_dirs.rs:74-82`), can still have
  **real, non-linked data** sitting only under an old per-channel path.

So the honest state is: for any channel created after the fix, the transcript half of this is
provably safe to ignore when deleting. The disk-bloat half is not solved, and pre-fix or
failed-link channels are a genuine exception that inspection can catch (see §4) but automated
cleanup cannot yet assume away.

## 4. What this means for cleanup — the checklist that came out of this

Before deleting an old channel's `identities/<uuid>/claude/` directory:

1. **Check whether `projects/` is a junction or a real directory, AND where the junction
   actually points**, don't assume either. `ensure_history_link` itself
   (`agentmux-common/src/data_paths.rs`) only calls a link "correct" when it resolves to
   exactly `identity_history_dir()`'s current value — checking `LinkType` alone isn't the same
   check the code uses, and passes for a stale/misdirected link too (Codex P2 on #3123). On
   Windows: `(Get-Item <path>).Target` (or `junction`'s own `get_target`) compared against the
   current `identity_history_dir()` path; on Unix, `readlink` compared the same way.
2. **If it's a junction/symlink resolving to the current target** — safe to delete outright.
   It's an access path, not data; the target is untouched and still reachable from the current
   live channel.
3. **If it's a real directory, or a junction pointing somewhere else** (pre-fix data, a failed
   best-effort link, or a stale target from before some earlier relocation) — do not assume
   redundancy, and do not treat "I relaunched the app under that channel" as sufficient proof
   of safety on its own (Codex P1 on #3123). `link_history_if_isolated` calls `ensure_history_link`
   best-effort and only **logs** a failure — a name collision at the migration target (`dest.exists()`
   in `ensure_history_link`) leaves the colliding file un-migrated and the source directory
   non-empty, so `remove_dir` fails, the directory stays real, and nothing surfaces that to a
   human watching for it. After relaunching to trigger the migration, **re-check step 1** —
   confirm `projects/` actually became a correctly-targeted link and that no stray files remain
   in the old location — before deleting anything. If it didn't migrate cleanly, diff/merge the
   leftover files into the global location by hand instead.
4. **The other files directly under `identities/<uuid>/claude/`** (`.claude.json`,
   `.credentials.json`) are genuinely per-channel and not mirrored anywhere. That's fine to
   lose once the channel is confirmed dead (no running instance) — a relaunch under that
   channel, if it ever happened again, just redoes the one-time OAuth/trust flow, the same
   accepted per-build-identity pattern already described in
   [retro-keychain-prompt-recurs-per-build-identity-2026-08-21.md](retro-keychain-prompt-recurs-per-build-identity-2026-08-21.md).

For the three specific channels that prompted this retro (`0d9e24f4`, `13d6e7d1`, `7c55a408`
— all created 2026-09-04 through 09-08, well after the 08-16 fix): all three verified as
healthy junctions to the same file. Deleting the two non-live ones is confirmed safe.

## 5. Take-away

A directory's *own* mtime is not evidence about what's inside it once junctions/symlinks are
in play — the parent dir's mtime reflects its own real children, not the resolved target of a
reparse point living inside it. The 10-second version of the fix (`readlink`/`LinkType` before
drawing any conclusion from what looks like duplication) would have resolved this without
needing to pause and investigate. Filed as a retro anyway because the *first* time this
pattern is encountered on a given codebase, it's genuinely not obvious that a directory that
looks freshly-populated is actually just a fresh door into old, unmoved data — and the disk-
bloat half of the story (§3) is real and still needs the pruner this repo already spec'd.
