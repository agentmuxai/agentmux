# SPEC: Launch context — host agents work in their own workspace, the card lists every startup file, and CLI upgrades are shown

**Status:** LC1 (§3) implemented in this PR; LC2-LC5 proposed.
**Date:** 2026-09-30
**Verified against:** `agentmux` `main` @ `48cc6fdbc` (§1-§4) and `3fcd1496a` (§6). Paths are relative
to the repo root.
**Related:**
- `SPEC_CONTEXT_DELIVERY_2026_09_30.md` (#4034): the per-item delivery
  card. This spec adds a new item kind to it and depends on its CD2/CD3
  (per-item fields, bodies on expand).
- `SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md`: the startup-file and hook
  delivery paths. Its table at :180 ("new session | startup file only")
  is stale; see §2.3.
- `SPEC_SYSTEM_TIER_GLOBAL_MEMORY_SEEDING_2026_09_15.md`: the Operator
  Config seed this spec adds an entry to.
- `docs/reports/REPORT_OUT_OF_WORKSPACE_CLONE_AUDIT_2026_09_05.md`: the
  in-workspace clone convention, which no agent is told today.

## 0. The ask

From the owner, 2026-09-30, after checking what a Claude agent had been
given at the start of a continued session:

> "does the global agentmux instructions include the instruction to always
> work out of your workspace?" … "lets add that for Host agents, they
> should clone out of their workspace. we also want to include all the
> files that you received, find where in the code that happens."

Then, the same day, relaying Agent1's research on CLI pinning:

> "we want to add another element … Users need to know when agent clis
> are being upgraded (similar to the other injections)"

Three changes:
1. **Workspace rule (§3).** Host agents are told to clone and work inside
   their own workspace directory.
2. **Every startup file in the card (§4).** The "Given to the agent" card
   lists everything the agent received at launch, not just what the
   SessionStart hook carried.
3. **CLI upgrades are shown (§6).** When an agent's CLI is installed or
   comes back up on a different version, the pane says so.

"Clone out of their workspace" is read as "clone into, and work out of,
their own workspace", matching the audit report's convention. See §8 Q1.

## 1. Terms

- **Workspace:** the agent instance's working directory,
  `~/.agentmux/agents/<instance-slug>/` (e.g. `clamk-0612a`). The frontend
  builds the path (`frontend/app/view/agent/agent-model.ts:480-495`, slug
  from `defaults/instance-slug.ts:73`), srv claims it atomically
  (`allocate_agent_workdir`, `crates/srv/src/server/app_api/agent_define.rs:357`)
  and stores it as the instance's `working_directory`. The agent gets it only
  as its launch cwd; no env var names it.
- **Host agent:** an agent whose CLI runs as a process on this machine.
  `AgentDefinition.agent_type` (`crates/srv/src/backend/storage/agents.rs:67`)
  takes three values: `"host"`, `"standalone"` (the stored default,
  `:228-230`) and `"container"` (Docker, `:114-129`). **Host = anything
  other than `"container"`.** This is unrelated to the jekt "host tier"
  (same-machine HMAC trust).
- **Startup file:** a file the provider CLI reads by itself at session
  start, as opposed to content AgentMux pushes through a hook or a hidden
  turn.

## 2. What happens today (verified)

### 2.1 What a new Claude session receives

Observed on agent `clamk-0612a` (Claude, host, foreign `CLAUDE.md`), and
traced in code:

| # | Content | How it arrives | Written by | In the card? |
|---|---|---|---|---|
| 1 | `~/.agentmux/agents/CLAUDE.md` (jekt security rules, PR identity tag) | Claude Code's ancestor-directory `CLAUDE.md` discovery | **No code.** Hand-maintained; its header calls itself the authoritative agent-facing copy since the repo `CLAUDE.md` was removed (#3403) | no |
| 2 | `<workspace>/CLAUDE.md` | Claude's project `CLAUDE.md` | `write_claude_md_respecting_ownership` (`crates/srv/src/backend/agent_config.rs:1401`). Rewritten only if it carries `CLAUDE_MD_MANAGED_MARKER` (:936) or doesn't exist | no |
| 3 | `<workspace>/.claude/AGENTMUX_MEMORY.md` | `@.claude/AGENTMUX_MEMORY.md` import appended to a foreign `CLAUDE.md` (:941, :962, :1509-1523) | same function, every launch | no |
| 4 | Global Memory (the 3 Operator Config entries, plus any workspace entries) | inside #2 or #3, as `# Memory` | `inject_global_bundles` (`crates/srv/src/server/editor_handlers.rs:93-116`) or `write_agent_config_files` (`app_api/agent_open.rs:1057-1066`), both via `format_global_bundle_block` (`backend/storage/bundles.rs:184`) | no (as a file) |
| 5 | Global + Personal Memory again | SessionStart hook, `agentmux-bashwrap sessionstart --part 1..8` | `compose_delivery` (`server/memory_delivery_handlers.rs:340`), `global_entries` (:364), `personal_entries_in` (:387) | **yes, the only thing in the card** |
| 6 | Skills index `# Available Skills` | inside #2/#3 | `build_config_files` (`agent_config.rs:101-117`) | no |
| 7 | Skill files `.claude/commands/*.md`, `.claude/skills/*/SKILL.md` | Claude's native skill/command listing | `agent_config.rs:139-158` | no |
| 8 | MCP tools (`agentmux` server) | `.mcp.json` | `write_mcp_json_respecting_user_servers` (`agent_config.rs:1222-1312`) | no |
| 9 | `<CLAUDE_CONFIG_DIR>/CLAUDE.md` (user-level) | Claude's user memory | placeholder seeded by `seed_claude_md_placeholder_if_missing` (`crates/srv/src/backend/providers.rs:812-869`) | no |
| 10 | Continuation packet | prefixed to the first user message | `build_continuation_packet` (`backend/continuity.rs:262-337`) | no (label only; #4034 CD4 covers it) |

The card (`frontend/app/view/agent/components/ContextDeliveryCard.tsx`)
is built from the `agentmux_memory_injected` frame that
`notice_frame` (`memory_delivery_handlers.rs:483`) appends after the hook
parts are acknowledged. So it can only ever show row 5. On the observed
session it said "3 items" while the agent had been given rows 1-10.

### 2.2 No workspace rule anywhere

Nothing the agent receives mentions where to clone or work. The Operator
Config seed (`crates/srv/operator-config-seed.json`, manifest version 5,
entries `operator-config-app-api`, `-environment-gotchas`,
`-rich-output`) doesn't. Neither do the shared `agents/CLAUDE.md` or the
per-agent `CLAUDE.md`. The convention exists only in
`REPORT_OUT_OF_WORKSPACE_CLONE_AUDIT_2026_09_05.md` §0 and
`docs/analysis/ANALYSIS_MULTI_AGENT_SESSION_AND_WORKDIR_ISOLATION_2026-07-29.md`.

### 2.3 Duplicates

- **Global Memory twice per new session.** Row 4 and row 5 are built
  from the same `global_bundle_sections` (`bundles.rs:223`), byte for
  byte by design (`bundles.rs:181-183`). `Reason::from_hook_source`
  (`backend/memory_delivery.rs:45-52`) delivers on `startup`, so a new
  session gets the preamble and every entry twice (≈2.3k tokens today).
  On `clear`/`compact` the re-delivery is intended.
  `SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md:180` says a new session gets
  the startup file only; the code disagrees.
- **Skills up to three times.** The frozen foreign `CLAUDE.md`'s old
  list, the `# Available Skills` index in `AGENTMUX_MEMORY.md`, and
  Claude's own listing of the skill files.
- **A stale foreign `CLAUDE.md`.** `clamk-0612a/CLAUDE.md` (dated
  2026-09-18) holds exactly what an older AgentMux wrote: a skills list
  and the import line, but no managed marker. It's treated as the user's
  file and never updated again.

### 2.4 Global Memory can't target an agent kind

`SeedEntry` (`backend/operator_config_seed.rs:55-73`) has only `id`,
`name`, `instructions`. `global_bundle_sections` includes every non-empty
global bundle. `Bundle.provider` / `instructions_by_provider` exist but
the Global Memory path never reads them.

## 3. Design: the workspace rule

### 3.1 Where it lives

A fourth Operator Config entry, `operator-config-workspace`, delivered to
**host agents only**. Operator Config is the right home: it's AgentMux's
own guidance, it's synced across versions, user edits are preserved
(`bundle_reseed_system_if_owned`, `bundles.rs:836-871`), and it reaches
every provider that has a startup file (all but Kimi).

Rejected:
- **The shared `agents/CLAUDE.md`.** Claude only, hand-maintained, and
  not shipped with AgentMux, so other installs never get it.
- **An unconditional entry that says "if you're a host agent".** It costs
  container agents tokens for a rule that doesn't apply to them, and the
  agent can't tell which kind it is (no env var says so).

### 3.2 Targeting by agent kind

1. `SeedEntry` gains optional `agent_types: ["host"]`. Absent means all
   agents (the three existing entries are unchanged).
2. **The target lives in the manifest only, not in `db_bundles`.** A new
   column bumps the object schema version, and `check_schema_compat`
   (`backend/storage/migrations.rs:2342`) then refuses to open the store
   in any older build, and the stable install and a `task dev` build
   share one store.db. The filter looks the bundle id up in the embedded
   manifest, so a local edit (same id) keeps its target, and a system row
   this build's manifest doesn't know (a newer build seeded it) is always
   delivered.
3. One shared filter, `operator_config_seed::global_bundles_for_agent(bundles,
   agent_mode)`, runs before `format_global_bundle_block` /
   `global_bundle_sections` at every composition point, so the startup
   file, the hook and the fallback stay byte-identical. The kind is
   `container` for `agentMode == "container"`, otherwise `host`
   (`agent_kind`).
4. Where each point gets the kind:
   - `agent.open` (`write_agent_config_files`): `agent.agent_type`;
   - Launch (`writeagentconfig`): a new optional `agent_type` on
     `CommandWriteAgentConfigData`, which the frontend fills from
     `agentMode`; absent means host;
   - SessionStart hook (`global_entries`): the block's `agentMode` meta
     (`agent_mode_of_block`), `"host"` if the block can't be read;
   - fallback (`globalmemory:sections`): a new optional `block_id`, read
     the same way; absent means host.
5. **Deferred:** scoping the user's own (workspace) entries. That needs the
   column, and so a schema change planned around the store-sharing
   constraint above.

### 3.3 The entry's text

Id `operator-config-workspace`, name `AgentMux Operator Config: Your
workspace`, `agent_types: ["host"]` (`crates/srv/operator-config-seed.json`,
manifest v6). The "host agents" scope is stated in the body rather than
the name, like the other entries' opening line:

> This entry is maintained by AgentMux itself (Operator Config) and is the
> same across every AgentMux deployment. It is given only to agents that
> run as a process on this machine (host agents), not to container agents.
>
> Your working directory at launch is your own workspace:
> `~/.agentmux/agents/<your instance>/`. Its path is also in the
> `AGENTMUX_AGENT_WORKDIR` environment variable, so you can find it again
> after changing directory.
>
> - **Clone every repository you work on inside your workspace**, for
>   example `<workspace>/agentmux`, and keep its branches, builds and
>   scratch files there.
> - **Don't clone into, build in, or edit another agent's workspace**, your
>   home directory, or a shared checkout elsewhere on the disk. Another
>   agent may be using it, and its owner can't see a change you make there.
> - **For a second checkout of a repository you already have, clone again
>   inside your workspace.** `git clone --reference <your first clone>
>   <url> <dir>` saves disk. Don't use `git worktree` for this.
> - **Temporary files** belong in your workspace or the system temp
>   directory, never in another agent's workspace.
>
> If a task needs a file outside your workspace, read it where it is, and
> ask before changing it.

### 3.4 `AGENTMUX_AGENT_WORKDIR`

The rule points at the workspace, but an agent that has `cd`'d away (or
was resumed with a different cwd) has no reliable way to find it. srv
sets `AGENTMUX_AGENT_WORKDIR` to the block's `cmd:cwd` (the resolved
workspace) in `build_persistent_spawn_env`
(`carry_agent_workdir_env`, `crates/srv/src/server/agent_handlers/input.rs`),
the env builder every spawn path shares. It is server-set: a `cmd:env`
copy never wins. Host agents only; it is removed for a container agent
and is on `CONTAINER_ENV_DENYLIST`, because the host path means nothing
inside the image.

## 4. Design: every startup file in the card

### 4.1 A launch manifest

When srv writes an agent's config at launch (both paths:
`writeagentconfig` in `editor_handlers.rs:205-394` and
`write_agent_config_files` in `agent_open.rs:1007-1245`), it also records
a **launch manifest**: the list of startup files the provider will read,
in the order it reads them. One item per file:

| Field | Meaning |
|---|---|
| `kind` | `startup_file` (new item kind, alongside #4034's `global_memory`, `personal_memory`, …) |
| `role` | `instructions` / `instructions_import` / `ancestor_instructions` / `user_instructions` / `skill` / `mcp_servers` |
| `path` | absolute path |
| `owner` | `agentmux` (managed marker or written this launch) / `user` (foreign) / `external` (no AgentMux code writes it, e.g. the shared `agents/CLAUDE.md`) |
| `bytes`, `tokens` | size as read (tokens = chars/4, as today) |
| `sha256` | content hash at launch |
| `contains` | for AgentMux-written files, the sections inside: `global_memory` (with entry names), `skills_index`, `soul`, `agent_md` |
| `duplicates` | ids of other items in the same card carrying the same content (§4.4) |

The list is **provider-specific**, from a new
`startup_file_candidates(provider, workdir, config_dir)` in
`backend/providers.rs`, next to `startup_instructions_filename`. For
Claude:
- `<workdir>/CLAUDE.md`, `<workdir>/CLAUDE.local.md`;
- each ancestor directory's `CLAUDE.md` / `CLAUDE.local.md`, up to the
  filesystem root (this is how the shared `agents/CLAUDE.md` is found);
- `@path` imports inside any of those, followed recursively to the depth
  Claude Code allows, relative to the importing file;
- `<CLAUDE_CONFIG_DIR>/CLAUDE.md`;
- the skill files AgentMux wrote, as one grouped item with a count;
- `.mcp.json` servers, as one item: **server names and tool count only,
  never file contents** (it holds `AGENTMUX_JEKT_KEY` and other keys).

For codex/gemini/qwen/pi, their one instructions file
(`providers.rs:345,377,426,555`); ancestor discovery per provider is a
follow-up (§8 Q4). Kimi gets an empty manifest and a card that says so.

This mirrors the provider's documented loading rules; it is what the
provider **should** read, not proof that it did. The card says
"files at launch", not "files read".

### 4.2 When it is sent

srv stores the manifest under the launch and emits it with the startup
delivery:
- if the SessionStart hook fires with `source=startup`, the manifest's
  items are added to that delivery's `agentmux_memory_injected` frame,
  before the hook's own items;
- for providers without the hook (everything but Claude), srv emits the
  frame by itself when the first turn is sent, with the manifest only.

A resume (`source=resume`) gets no card today and still gets none; the
files haven't changed the agent's context. `/clear` and compaction get a
card with the hook items plus a single row "Startup files: unchanged
since launch (N files)", expanding to the list, because Claude keeps
them in context.

### 4.3 How the card shows it

Header: "Given to the agent · new session · 11 items · 2 duplicated".
Two groups:

- **Startup files** — one row per file: name, path (shortened to `~`),
  owner chip (`AgentMux` / `yours` / `external`), size and tokens.
  Expanding shows the content snapshot (#4034 CD3 storage and RPC),
  except `.mcp.json`. "Open file" opens it in an Editor pane.
- **Memory** — the hook's items, as #4034 describes.

The owner chip answers "why is this here and who can change it":
`external` rows say where they come from ("hand-maintained, not written
by AgentMux").

### 4.4 Duplicates are shown, then removed

The manifest makes duplicates visible (`duplicates` field, a "duplicate"
mark on each row). Then three fixes remove them:

1. **Global Memory on startup.** On `source=startup`, the hook skips the
   Global section when the manifest shows the startup file carries it
   (the import line is present, or AgentMux owns `CLAUDE.md`). If a user
   removed the import, the hook still delivers it. Personal Memory is
   unaffected (the startup file never carries it). Updates
   `SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md:180` to match.
2. **Skills index for Claude.** Claude lists `.claude/commands` and
   `.claude/skills` natively, so the `# Available Skills` index is dropped
   from Claude's startup file. Other providers keep it.
3. **Legacy foreign `CLAUDE.md`.** A `CLAUDE.md` whose content is exactly
   what an older AgentMux wrote (a `# Available Skills` list plus the
   managed import line, nothing else) is adopted as AgentMux-owned: its
   content is replaced with the managed file and the marker, and the old
   one is kept as `.claude/CLAUDE.md.pre-adopt`. Anything else stays
   foreign.

## 5. Tests

- **Seeding:** the new entry seeds with `agent_types: ["host"]`; a
  changed `agent_types` re-seeds; a user edit to it is preserved.
- **Targeting:** `global_bundle_sections` with `host` includes the entry,
  with `container` excludes it; the startup file and hook output are
  byte-identical for each kind; a failed hook-side lookup includes all
  entries.
- **Env:** a host launch sets `AGENTMUX_AGENT_WORKDIR` to the resolved
  workdir; a container launch doesn't.
- **Manifest (Claude):** a temp tree with an ancestor `CLAUDE.md`, a
  foreign project `CLAUDE.md` with the import, a nested `@import`, and a
  config-dir `CLAUDE.md` yields the files in Claude's order, with correct
  owners; an import cycle terminates; `.mcp.json` yields names and a
  count, and its body is never stored (assert no key material in the
  stored delivery).
- **Duplicates:** startup with the import present → hook has no Global
  section and the card shows none duplicated; import removed → hook
  carries Global; `clear`/`compact` unchanged.
- **Legacy adopt:** the exact legacy shape is adopted with a backup; one
  extra line keeps it foreign.
- **Live check:** a new Claude host agent, a Codex agent and a container
  agent each show a card listing their real files; the Claude card shows
  the shared `agents/CLAUDE.md` as `external`; the host agents have the
  workspace entry and the container agent doesn't.

## 6. Design: CLI upgrades are shown

Research by Agent1 (2026-09-30), re-checked against `3fcd1496a`.

### 6.1 How a repin reaches an agent today

- **The pin is compiled in.** `ProviderDefinition.pinned_version`
  (`crates/srv/src/backend/providers.rs:113`; Claude `"2.1.285"` at :295,
  mirrored in the frontend catalog). A repin reaches a user only with a new
  AgentMux build.
- **Installs are shared, one directory per version.**
  `<shared>/cli/<provider>/<pinned_version>/` (`backend/cli_install.rs:92`),
  counted only once `.agentmux-install-complete` is written (:114-126),
  behind a cross-instance lock. A new pin installs next to the old one.
  Old version directories are never pruned.
- **The version is chosen when the CLI process spawns**, never for a
  running process. The frontend's `resolveCliBin`
  (`frontend/app/view/agent/agent-launch-env.ts:108-120`) calls
  `ResolveCli` with a 300 s timeout, which npm-installs the pinned version
  if it's missing (`server/cli_handlers.rs:180-215`). A Claude persistent
  pane keeps its binary until its process respawns (restart, reconnect,
  reopen, app restart); the conversation continues via `--resume`.

### 6.2 What the user is told

| Case | Told? |
|---|---|
| Launch from the picker, CLI not installed | Yes: the card's "Click to install" and `AgentInstallModal` with progress. |
| A pane restored at startup (already in the layout) | **No.** `ResolveCli` installs silently; the pane is just slower to start. The only trace is srv's log line `"CLI installed (npm)"` (`cli_handlers.rs:208`). |
| The agent comes back on a different version (e.g. 2.1.218 → 2.1.285) | **No,** on any path. The Toolchain pane shows the current version if you look. |
| "Update available" badge | Fires only for `behind-pin` (`providers/version-drift.ts:58-66,106`), i.e. an *older* install found, which only the legacy per-AgentMux-version layout produces. Never for a fresh instance. |
| The CLI updating itself | Unknown. Claude Code's auto-updater is disabled only in `docker/Dockerfile.agent-agentmux:98` (`DISABLE_AUTOUPDATER=1`); srv sets nothing for host agents, so an install may drift past the pin unnoticed. |

### 6.3 Design

The notices are pane rows in the same family as the delivery card (§4.3):
persisted frames in the block's output, so they replay with the
transcript, and never read by the digest (#4034 §3.6). They are for the
user; the agent's context is unchanged (§8 Q6).

1. **Installing notice.** `CommandResolveCliData` gains an optional
   `block_id`. When `ResolveCli` has to install, srv appends an
   `{"type":"system","subtype":"agentmux_cli_install"}` frame to that
   block: `state: "installing"`, `provider`, `version`, then `"installed"`
   (with seconds taken) or `"failed"` (with the error). The pane renders
   one row that updates in place: "Installing Claude Code 2.1.285…" →
   "Installed Claude Code 2.1.285 (14 s)". The picker path keeps its
   modal and gets the row too.
2. **Version-change notice.** srv keeps, per agent, the CLI it last ran:
   `{provider, version, path, at}`, in the agent's existing launch-state
   JSON (no schema change; §3.2 says why that matters).
   - **Source of the version:** for Claude, the `claude_code_version` of
     the CLI's own `system/init` frame, which covers every spawn path
     (Launch, restore, reconnect, eager resume) and reports the version
     that actually runs. For the others, the version `ResolveCli` /
     `find_installed` resolved at spawn (`get_cli_version`,
     `cli_handlers.rs:787`). To confirm in LC4: every Claude spawn path
     emits `init` before the first turn.
   - **When it differs from the record,** srv appends an
     `agentmux_cli_version_changed` frame `{provider, from, to, pinned}`
     and updates the record. The row: "Claude Code updated: 2.1.218 →
     2.1.285". A first-ever run (no record) emits nothing.
   - **Running version ≠ pin** (a self-update, or a stale install) adds
     "not the version AgentMux pins (2.1.285)" to the row, as a warning.
3. **Auto-updater off for host Claude agents.** Set
   `DISABLE_AUTOUPDATER=1` in the host spawn env, as the container image
   already does, unless `cmd:env` sets it: the pin is the tested version,
   and step 2's "≠ pin" warning only means something if nothing else moves
   the binary. First confirm whether the CLI self-updates inside
   AgentMux's install directories at all (§8 Q7).
4. **Pruning.** After a successful install, delete that provider's
   version directories that are neither the pin nor used by a running
   process of any instance, keeping the newest one before the pin for
   rollback. Uses the same cross-instance lock as install.

### 6.4 Tests

- `ResolveCli` with a `block_id` and a missing install emits
  `installing` then `installed`; an npm failure emits `failed`; an
  existing install emits nothing.
- Two spawns on the same version emit no change notice; a different
  version emits one with the right `from`/`to` and updates the record; a
  first-ever spawn emits none; a version ≠ pin sets the warning.
- Replay renders both rows from persisted frames; the digest ignores them.
- The host spawn env carries `DISABLE_AUTOUPDATER=1` for Claude unless
  `cmd:env` overrides it; the container env is unchanged.
- Pruning keeps the pin, the previous version and any directory in use.
- **Live:** bump the pin in a dev build and restore a pane from the old
  build: the pane shows the install row, then the version-change row.

## 7. Phases

| Phase | What ships | Size |
|---|---|---|
| **LC1** | Workspace rule: `agent_types` on seed entries and bundles, filtered `global_bundle_sections`, the new entry (manifest v6), `AGENTMUX_AGENT_WORKDIR` (§3). Independent of #4034. | medium |
| **LC2** | Launch manifest and startup-file rows in the card (§4.1-4.3). Needs #4034 CD2; bodies need CD3. | medium |
| **LC3** | Duplicate removal (§4.4): hook skips Global on startup, skills index off for Claude, legacy adopt. | small |
| **LC4** | CLI upgrades shown (§6.3 steps 1-2): install notice, version-change notice, the per-agent record. | medium |
| **LC5** | Auto-updater off for host Claude agents, and pruning old version dirs (§6.3 steps 3-4). | small |

LC1 is the owner's direct ask and ships first. LC4 is independent of the
others and can ship in parallel. LC3 should land after LC2,
so the card can show the duplicates going away.

Every phase ships its docs (§9).

## 8. Open questions

1. **"Clone out of their workspace."** Read as "clone into, and work out
   of, their own workspace" (§0). If the intent is the opposite (clone
   *outside* it), §3.3 changes.
2. **Should the rule be enforced, not just stated?** E.g. the bashwrap
   PreToolUse hook warning on `git clone` into a path outside the
   workspace. Recommended: not in this spec; state it first, measure with
   the audit report's method, then decide.
3. **Container agents:** what's their equivalent rule (their mounted
   volume)? Out of scope; `agent_types` makes it easy to add later.
4. **Ancestor discovery for other providers** (Codex reads `AGENTS.md`
   up the tree too). Follow-up; LC2 lists their one file.
5. **Should the shared `agents/CLAUDE.md` become AgentMux-owned** (a
   Global Memory entry, shipped with AgentMux) instead of a hand-maintained
   file? It's the largest startup file and reaches only Claude agents on
   this machine. Separate decision; LC2 makes its cost visible.
6. **Tell the agent about a CLI version change too?** Recommended: no.
   The notice is for the user; a changed CLI doesn't change what the agent
   should do, and a line in its context costs tokens every session.
7. **Does Claude Code self-update inside AgentMux's install directories?**
   Check on a host agent before LC5 (step 3). If it doesn't, step 3 is
   still worth doing as a guard, but it's no longer urgent.

## 9. Docs to update

User docs live in `agentmuxai/agentmux-docs` (`src/content/docs/`).

| Doc | Change | Phase |
|---|---|---|
| `memory.md` | Global Memory entries can apply to host agents, container agents or all; list the Operator Config entries, including the workspace one. | LC1 |
| agent environment variables page | Add `AGENTMUX_AGENT_WORKDIR`. | LC1 |
| `memory.md`, "How Memory bundles are reached" | The card lists startup files with their owner; startup no longer duplicates Global Memory. | LC2, LC3 |
| `agentmux` `docs/specs/SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md` | Fix the :180 table (today: startup file + hook; after LC3: file only when the import is present). | LC3 |
| `agentmux` `docs/specs/SPEC_CONTEXT_DELIVERY_2026_09_30.md` | Add the `startup_file` item kind and point at §4 here. | LC2 |
| `agentmux` `docs/specs/SPEC_SYSTEM_TIER_GLOBAL_MEMORY_SEEDING_2026_09_15.md` | Document `agent_types` on seed entries. | LC1 |
| agent providers / toolchain page | When a CLI is installed or updated, and what the pane shows; the auto-updater is off for host Claude agents. | LC4, LC5 |
