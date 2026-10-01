# SPEC: Launch context — host agents work in their own workspace, the card lists every startup file, and CLI upgrades are shown

**Status:** active — LC1 (§3) implemented (#4113); LC4 (§6.3 steps 1-2) implemented (#4128); LC2 (§4.1-4.3, Claude) implemented (#4129); LC3 (§4.4) implemented; LC5 proposed.
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
manifest v7). The "host agents" scope is stated in the body rather than
the name, like the other entries' opening line.

The launch directory isn't always a private folder AgentMux allocated: a
definition can set its own `working_directory` (a project folder), and
`agent.open` with a blank one falls back to `~/.agentmux/agents/<slug>`,
shared by every launch of that name (`agent_open.rs:1023-1029`). So the
text defines the workspace as the launch directory, says it may be a
chosen project folder, and tells an agent already in the repository it
was asked to work on to work there instead of nesting a clone (Codex P1
on #4113). Gating delivery on "allocated" instead was rejected: the
hook and fallback paths only know the block, and the three launch paths
classify the directory differently.

> This entry is maintained by AgentMux itself (Operator Config) and is the same across every AgentMux deployment. It is given only to agents that run as a process on this machine (host agents), not to container agents.
>
> Your working directory at launch is your workspace. Its path is also in the `AGENTMUX_AGENT_WORKDIR` environment variable, so you can find it again after changing directory. Usually it is a folder AgentMux created for you under `~/.agentmux/agents/`, but it can also be a project folder chosen for this agent.
>
> - **If your workspace already is the repository you're asked to work on**, work in it directly. Don't clone a second copy inside it.
> - **Otherwise, clone every repository you work on inside your workspace**, for example `<workspace>/agentmux`, and keep its branches, builds and scratch files there.
> - **Don't clone into, build in, or edit another agent's workspace** (another agent's folder under `~/.agentmux/agents/`), or a checkout elsewhere on the disk that you weren't pointed at. Another agent may be using it, and its owner can't see a change you make there.
> - **For a second checkout of a repository you already have, clone again inside your workspace.** `git clone --reference <your first clone> <url> <dir>` saves disk. Don't use `git worktree` for this.
> - **Temporary files** belong in your workspace or the system temp directory, never in another agent's workspace.
>
> If a task needs a file outside your workspace, read it where it is, and ask before changing it.

### 3.4 `AGENTMUX_AGENT_WORKDIR`

The rule points at the workspace, but an agent that has `cd`'d away (or
was resumed with a different cwd) has no reliable way to find it. srv
sets `AGENTMUX_AGENT_WORKDIR` to the block's `cmd:cwd` (the resolved
workspace) in `build_persistent_spawn_env`
(`carry_agent_workdir_env`, `crates/srv/src/server/agent_handlers/input.rs`),
the env builder of the persistent and per-turn spawn paths, and in the
ACP (`AcpController::spawn_env`) and app-server controllers, which build
their own (Codex P2 on #4113). It is server-set: a `cmd:env`
copy never wins. Host agents only; it is removed for a container agent
and is on `CONTAINER_ENV_DENYLIST`, because the host path means nothing
inside the image.

## 4. Design: every startup file in the card

### 4.1 The startup files, as the CLI sees them

**As built (LC2), the list is computed when the session starts, not
recorded at launch.** Neither launch path is a good place to record it:
`writeagentconfig` knows no block, and the two paths share only leaf
writers keyed by the workdir. The SessionStart hook, by contrast, runs
inside the CLI. Its stdin carries the session's `cwd`, and its env has
`CLAUDE_CONFIG_DIR`. So `agentmux-bashwrap sessionstart` sends both with
each part request (`crates/bashwrap/src/sessionstart.rs`), and srv lists
the files from where the CLI actually ran (`backend/startup_files.rs`,
`claude_startup_files`). A hook binary older than srv sends no `cwd`, and
the card simply shows no startup files.

One item per file, with the memory items' fields plus:

| Field | Meaning |
|---|---|
| `kind` | `startup_file` |
| `role` | `user_instructions` / `instructions` / `instructions_import` / `skills` / `mcp_servers` |
| `path` | absolute path; the `name` is shortened to `~/…` |
| `owner` | `agentmux` (the managed marker, the config-dir placeholder, or `AGENTMUX_MEMORY.md`) / `user` (in the workspace, not managed) / `external` (outside the workspace and not AgentMux's, e.g. the shared `agents/CLAUDE.md`) |
| `size_bytes`, `tokens` | of the text as read (tokens = chars/4, as today) |
| `count` | skills and MCP servers: how many are listed (they carry no text, size 0) |
| `contains` | AgentMux sections the file carries: `global_memory`, `skills_index` |

For Claude, in the order the CLI loads them:
- `<CLAUDE_CONFIG_DIR>/CLAUDE.md`, else `~/.claude/CLAUDE.md`;
- each folder from the filesystem root down to `cwd`, excluding the root
  itself: its `CLAUDE.md`, `.claude/CLAUDE.md`, `CLAUDE.local.md`. This
  is how the shared `agents/CLAUDE.md` is found;
- right after each file, its `@path` imports: relative to the importing
  file, or `~/`; not inside fenced or inline code; up to 5 hops; each file
  once, so a cycle ends;
- the skill listing (`.claude/commands/*.md`, `.claude/skills/*/SKILL.md`)
  as one item with a count;
- `.mcp.json`'s servers as one item, **by name only**. The file holds the
  agent's signing keys, so its text never leaves `startup_files.rs`.

This mirrors Claude Code's documented loading rules; it is what the CLI
**should** have read, not proof that it did.

**Not built: other providers.** codex, gemini, qwen and pi have no
SessionStart hook, so they get no card at all today. Listing their one
instructions file needs srv to emit a frame by itself on the first turn
(§8 Q4).

### 4.2 When it is sent

On a new session (`source=startup`) only. The startup items come first in
that delivery's `agentmux_memory_injected` frame, then the hook's memory
items. `/clear` and compaction get no startup rows: the files are still in
context and nothing about them changed. A resume gets no card, as before.

Not built: a "Startup files: unchanged since launch" row on `/clear` and
compaction. It isn't needed for the ask and can follow.

### 4.3 How the card shows it

One card, "Given to the agent · new session · N items", with the startup
files first. Each row shows:
- 📄 and the name;
- an owner chip, `AgentMux`, `Yours` or `Hand-maintained`, with a
  tooltip explaining who writes the file;
- `N listed` instead of a size for the skill and MCP listings;
- a muted `+ Global Memory` mark on a file that carries the Global Memory,
  since the hook delivers it too (§4.4).

Bodies on expand wait for #4034's CD3, which still needs the owner's
decision.

### 4.4 Duplicates are shown, then removed

The LC2 card shows duplicates: a `+ Global Memory` mark on a startup file
that carries it. LC3 then removes them in three steps:

1. **Global Memory on startup.** On `source=startup`, the hook leaves the
   Global section out when a startup file the CLI loaded carries it, i.e.
   a listed file whose `contains` has `global_memory`. That covers an
   imported `AGENTMUX_MEMORY.md` and an AgentMux-owned `CLAUDE.md`. A
   user who removed the import gets it from the hook again.
   - The hook's header says so: "Your Global Memory is already in your
     startup instructions, so it isn't repeated here."
   - The card still lists each Global entry, with `via: "startup_file"`,
     size 0 (the file's row counts it), shown as "in startup file".
   - With nothing left for the hook to carry (no Personal Memory), the
     delivery has no parts, so its card is written on the first part
     request (`send_partless_notice`).
   - Personal Memory is unaffected (no startup file carries it).
   - `/clear` and compaction are unchanged: there the hook still
     re-delivers.
2. **Skills index for Claude.** Claude Code lists `.claude/commands/*.md`
   and `.claude/skills/*/SKILL.md` by itself. So Claude's `# Available
   Skills` index now carries only skills left without a file of their
   own: empty content, a prompt skill with no usable trigger, or a command
   whose file a later skill's command overwrites (same trigger, compared
   case-insensitively; the skill store doesn't enforce unique triggers).
   Dropping those would hide them entirely. Other providers, and Claude
   aliases resolve to Claude first, read only their instructions file and
   keep the full index. The rule is `skills_with_their_own_file` in Rust
   `build_config_files`, mirrored by `commandKey`/`hasOwnFile` in the
   TypeScript `buildConfigFiles`. The instructions file is now written even
   when empty, since srv injects the Global Memory into it.
3. **Legacy foreign `CLAUDE.md`.** A `CLAUDE.md` whose content is exactly
   what an older AgentMux wrote is adopted as AgentMux-owned. "Exactly"
   means, in order and ignoring blank lines: the `# Available Skills`
   heading, its usage line, at least one skill row exactly as rendered
   (`- **name**`, optional ` (trigger: /x)`, optional ` — description`),
   then optionally the managed import (`is_legacy_agentmux_claude_md`,
   `is_generated_skill_row`). A symlinked `CLAUDE.md` is never adopted or
   written through, and the backup path is symlink-checked.
   - It is rewritten as the managed file, with the marker.
   - The original is kept in `.claude/CLAUDE.md.pre-adopt`, and a later
     adoption never overwrites that copy. A failed backup leaves the file
     as it is.
   - Anything else stays foreign. On this machine most agents' `CLAUDE.md`
     had this frozen shape, with a stale skills list.

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

1. **Installing notice.** `CommandResolveCliData` already carries the
   pane's `block_id` (`resolveCliBin` and the launch flow send it). When
   `ResolveCli` has to install, srv appends an
   `{"type":"system","subtype":"agentmux_cli_install"}` frame to that
   block: `state: "installing"`, `provider`, `version`, then `"installed"`
   (with seconds taken) or `"failed"` (with the error), all under one
   `install_id`. The pane renders one row that updates in place:
   "Installing Claude Code 2.1.285…" → "Installed Claude Code 2.1.285
   (14 s)". The picker's `AgentInstallModal` installs through
   `install.start`, which has no pane, so it keeps its own progress UI and
   gets no row.
2. **Version-change notice.** srv keeps, per agent, the CLI it last ran:
   `{provider, version, at}`, keyed by the block's `agentId`, in
   `agent-cli-versions.json` in the channel directory, next to the shared
   store that holds the agents (`backend/cli_notice.rs`, `record_dir`).
   Not `data_dir`: an installed build's is per AgentMux version, so an
   upgrade would start from an empty record and miss the very upgrade
   this reports (ReAgent P1 on #4128). There
   is no free-form launch-state field on `db_agents` (its launch state is
   typed columns), and a new column is a schema change (§3.2). A record
   that can't be read is treated as empty: at worst a notice is missed.
   - **Source of the version:** for Claude, the `claude_code_version` of
     the CLI's own `system/init` frame, which covers every spawn path
     (Launch, restore, reconnect, eager resume) and reports the version
     that actually runs. For the others, the version `ResolveCli` /
     `find_installed` resolved at spawn (`get_cli_version`,
     `cli_handlers.rs:787`). Confirmed: a stored Claude `init` frame
     carries `"claude_code_version":"2.1.285"`. The persistent reader
     (`persistent/spawn.rs`) is where every Claude spawn's stdout passes.
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
