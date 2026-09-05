# Report: Shared provider config — current state, how agents read it, and buildout

**Date:** 2026-09-05
**Status:** Draft — assessment + proposal. Nothing here is implemented. Contains
one verified latent defect (§3.1) that is independently actionable.
**Author:** Agent2
**Scope:** The "Claude Code — shared provider config" surface in Armory, the
on-disk config it points at, and the path by which a spawned agent actually
consumes it.

---

## 0. Short answer to "how are we doing"

**Thinner than the name suggests.** "Shared provider config" today is *one
hand-maintained file* — `~/.agentmux/shared/providers/claude/CLAUDE.md` —
surfaced in Armory as a **read-only preview**. There is no table, no schema, no
write path, no versioning, and no concept of shared provider *settings* beyond
that single Markdown file.

How agents read it is also different from every other shared Armory resource,
and that asymmetry is the main structural finding:

| Shared resource | How it reaches the agent |
|---|---|
| Global Memory (bundles) | **Composed** into `<work_dir>/CLAUDE.md` at launch |
| Skills | **Copied** into `<work_dir>/.claude/skills/…` at launch |
| MCP servers | **Composed** into `<work_dir>/.mcp.json` at launch |
| **Shared provider config** | **Not injected at all** — the Claude CLI reads it itself, off disk, via `CLAUDE_CONFIG_DIR` |

Everything else is materialized per-agent by AgentMux. The provider config is
the one case where AgentMux points an env var at a directory and trusts the
vendor CLI's own discovery. That's a legitimate design, but it has a
consequence: **if the file isn't there, the CLI silently falls back to the
operator's personal `~/.claude/CLAUDE.md`** — which is the exact failure
`SPEC_ISOLATE_HOST_CLAUDE_MD_2026_08_31.md` exists to prevent. §3.1 documents a
live code path where that seeding is skipped.

---

## 1. What exists today

### 1.1 The artifact

One file, no database:

- **Path:** `DataPaths::provider_auth_dir("claude")/CLAUDE.md` →
  `~/.agentmux/shared/providers/claude/CLAUDE.md`
  (resolver: `agentmux-common/src/data_paths.rs:444`; shared, deliberately
  channel- and version-independent).
- **Seeded content** when absent: `CLAUDE_MD_ISOLATION_PLACEHOLDER`
  (`agentmux-srv/src/backend/providers.rs:644-655`) — an HTML comment that
  explains itself and redirects the reader to Armory → Memory → Global.
- **No `db_*` table exists** for provider/shared config, and `settings.json`
  carries no provider keys.

### 1.2 The Armory surface

There is **no "Claude Code config" tab.** It lives under **Armory → Memory →
Global**, inside an "External Claude Code files" section, as one of two
read-only blocks (`frontend/app/view/brain/global-brain-manager.tsx`):

- `"Claude Code — shared provider config"` — caption *"Used by default spawned
  agents."*
- `"Claude Code — host CLI config"` (`~/.claude/CLAUDE.md`) — the file a Claude
  CLI running *outside* AgentMux reads.

Both are path + `<pre>` preview. No textarea, no save, no edit affordance of any
kind — read-only by construction, per `SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md`
§0/§2.3.

Worth reading that spec before touching this area: it went through **three
revisions correcting its own factual errors** (§5 — it originally displayed the
wrong file entirely; §7.2 — it wrongly claimed working-directory `CLAUDE.md`
files weren't part of the spawned-agent system). §7.3 states the section's
actual purpose plainly: *"External Claude Code files — managed by Claude Code
itself, not AgentMux. Shown for visibility only; not part of the Global Memory
composed above."*

### 1.3 RPC surface

- `getclaudeglobalconfig` (`rpc_types/commands.rs:254`), handler in
  `agent_handlers/memory.rs:240-250`. **Read-only — there is deliberately no
  write counterpart.**
- Sibling `getclaudehostconfig` for the ambient file.
- Frontend: `frontend/app/store/rpc-api/memory.ts:81-87`.
- Adjacent: CEF IPC `ensure_auth_dir`
  (`agentmux-cef/src/commands/platform.rs:79-121`) creates/returns the dir.

---

## 2. How an agent actually reads it

Two distinct spawn paths, and **they do not behave identically**.

### 2.1 App-API spawn (`agent_open.rs`)

1. Resolve `auth_dir` — identity-bound dir if the agent is bound to an Armory
   account, else `DataPaths::provider_auth_dir(provider)` (`agent_open.rs:338-345`).
2. **`prepare_provider_auth_dir(...)`** (`agent_open.rs:360`) → creates the dir
   **and seeds the placeholder `CLAUDE.md`**, fail-closed (spawn is blocked on
   error).
3. `env_vars["CLAUDE_CONFIG_DIR"] = auth_dir` (`agent_open.rs:366`).
4. Separately, `write_agent_config_files` composes Global Memory / skills / MCP
   into the *working directory* — a different set of files entirely.

### 2.2 UI "Launch" (`agent-model.ts`)

1. `getApi().ensureAuthDir(provider.id)` → CEF `ensure_auth_dir`
   (`agent-model.ts:519-522`).
2. `envVars[provider.authConfigDirEnvVar] = authDir`.
3. `RpcApi.WriteAgentConfigCommand(...)` → `editor_handlers.rs`, then block meta
   + controller spawn.

**This path never calls `agent_open.rs`, and therefore never seeds.** Verified
by exhaustive grep: `seed_claude_md_placeholder_if_missing` has exactly two
production callers — `agent_open.rs:360` and `identity/resolver/inject.rs:648`
(identity-bound). Everything else is tests.

### 2.3 Precedence, as actually implemented

- **Auth dir:** identity-bound account dir > shared `provider_auth_dir`.
- **Provider config content:** not merged by AgentMux at all — whatever is at
  `$CLAUDE_CONFIG_DIR/CLAUDE.md` is what the CLI reads. `CLAUDE_CONFIG_DIR`
  relocates the *entire* Claude home (`SPEC_PROVIDER_ISOLATION_2026_06_20.md`
  §5b), so there is no ambient+isolated merge — it's a replacement.
- **Working-directory content** (the separate pipeline): global brain prepended
  before per-agent memory; agent's own skill/MCP refs authoritative over legacy
  blobs; user hooks merged with AgentMux entries prepended.

---

## 3. Findings

### 3.1 🔴 Verified latent defect — the UI launch path skips isolation seeding

**`agentmux-cef/src/commands/platform.rs:113-120` calls `create_dir_all` and
nothing else.** It does not seed the placeholder. It is the dir-provisioning
step for the default UI launch path (`agent-model.ts:520`).

**Consequence:** on a host where `~/.agentmux/shared/providers/claude/CLAUDE.md`
does not yet exist, an agent launched from the UI gets `CLAUDE_CONFIG_DIR`
pointed at a directory with no `CLAUDE.md`. Claude Code's user-level discovery
then falls through to the operator's real `~/.claude/CLAUDE.md`, and the agent
silently inherits personal global instructions — precisely the isolation
failure `SPEC_ISOLATE_HOST_CLAUDE_MD_2026_08_31.md` was written to close, and
which that spec's own doc comment says was **verified happening live** on a real
machine (18+ isolated config dirs, none with a `CLAUDE.md`).

**Not currently firing on this host** — checked: the shared dir *is* seeded
(placeholder present, mtime 2026-08-31 17:42) and there is no
`~/.claude/CLAUDE.md` to inherit. So this is latent, not an active leak here.
The exposure window is: fresh install (or a newly-added provider dir, or a
manually-deleted `CLAUDE.md`) **+** first launch via the UI **+** an operator who
has a personal `~/.claude/CLAUDE.md`. Once any App-API spawn happens the dir is
seeded permanently, which is likely why this has gone unnoticed.

**Fix:** make seeding a property of *provisioning the dir*, not of one spawn
path. Either have `ensure_auth_dir` call the same
`prepare_provider_auth_dir` logic, or — better, since `platform.rs` lives in the
CEF host and the seeding logic lives in `agentmux-srv` — route the UI path's
dir provisioning through the srv RPC that already does it correctly, so there is
exactly one implementation. Two independently-maintained provisioning paths is
the actual root cause; adding a second seeding call would fix the symptom and
preserve the divergence.

### 3.2 🟡 No write path — "shared config" is hand-maintained

Editing requires opening the file on disk. No RPC, no editor, no validation, no
audit trail, no versioning — while the adjacent Global Memory bundles in the
*same tab* have all of those. This is a deliberate scoping decision
(`SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md` §0/§3), not an oversight, but
it's the main thing standing between "a preview" and "a shared provider config
feature."

### 3.3 🟡 "Provider config" means exactly one Markdown file

There is no shared `settings.json`, no shared hooks, no shared model/vendor
defaults, no per-provider policy. Every provider except Claude has *no* shared
config surface at all — the Armory block is Claude-only, and
`seed_claude_md_placeholder_if_missing` early-returns for any provider whose
`auth_dir_name != "claude"` (`providers.rs:679-681`), explicitly flagged as
"whether any other provider's CLI has an equivalent gap is unverified."

### 3.4 🟡 Identity-bound agents are invisible in the UI

The Armory block shows only the *default shared* dir. An identity-bound agent
reads a different per-identity dir, which nothing surfaces. Flagged as a known
gap in `SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md` §5 and never closed.
The tooltip admits it ("Identity-bound agents use a separate dir, not shown
here") — honest, but it means the block answers "what do agents read?" only for
the default case.

### 3.5 ⚪ Stale spec metadata

- `SPEC_ARMORY_DROP_HOST_CLI_CONFIG_BLOCK_2026_09_01.md` is marked
  `Status: Proposed` but the code implements it. *(Reported by survey, not
  independently verified — see §6.)*
- `SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md` §1 claims a
  `~/.agentmux/agents/CLAUDE.md` tier that does not exist; already documented as
  wrong in the 08-24 spec §1 but never corrected at source.

---

## 4. Proposed buildout

Ordered so each step is independently shippable and the riskiest assumption gets
tested first.

### Step 1 — Close the seeding divergence (do this regardless)

Independent of any feature work. Single provisioning path, seeding included, so
the isolation guarantee holds on every launch route. Small diff, real
correctness win, and it removes a trap for anyone building on top of this later.
Needs a regression test that asserts a UI-path launch leaves a seeded
`CLAUDE.md` — the current tests only cover the functions directly, never the
route that skips them.

### Step 2 — Decide the model before building UI

The genuine design question, and it should be answered explicitly rather than
drifted into:

**Option A — "It's Claude's file, we just show it."** Keep read-only. Maybe add
an "open in editor" button. Cheap, honest, matches the current spec chain's
stated intent. Ceiling: never becomes manageable shared config.

**Option B — AgentMux owns it, like Global Memory.** Give it a DB-backed source
of truth, an editor, versioning, and *compose* it to disk at launch the way
bundles already are. Consistent with every other shared resource — but it
collides with the file being hand-editable and CLI-owned, and needs an ownership
marker exactly like `CLAUDE_MD_MANAGED_MARKER`
(`SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md` already solved this problem
for working-directory files; reuse that pattern, don't reinvent it).

**Option C — Split the concepts.** Keep the CLAUDE.md read-only preview as-is,
and introduce a *separate*, properly-modelled "shared provider settings"
(defaults for model/vendor/base-URL/hooks per provider) with its own table and
editor. This is what "build out the shared provider config" most likely means in
practice, and it avoids fighting Claude Code over file ownership.

**Recommendation: C**, with A retained for the CLAUDE.md block. B is the most
consistent-looking but picks an ownership fight with the vendor CLI for a file
whose whole documented purpose is "managed by Claude Code itself, not AgentMux."

### Step 3 — Generalize beyond Claude

`seed_claude_md_placeholder_if_missing` hard-codes `auth_dir_name == "claude"`,
by explicit admission that other providers were never investigated. Before
shipping shared config as a general concept, someone has to answer per provider:
does its CLI have an equivalent user-level config discovery, and does
`<X>_HOME`/`<X>_CONFIG_DIR` relocate it the way `CLAUDE_CONFIG_DIR` does? Until
that's answered, "shared **provider** config" is really "shared Claude config."

### Step 4 — Surface identity-bound dirs (§3.4)

Only worth doing once Step 2 picks a model; otherwise it's more read-only
previews.

---

## 5. Suggested verification for Step 1

- Rust unit: `ensure_auth_dir`-equivalent provisioning on a temp `HOME` leaves a
  `CLAUDE.md` containing the placeholder.
- Rust unit: existing non-placeholder `CLAUDE.md` is **never** overwritten
  (already guaranteed by `path.exists()` at `providers.rs:697`; pin it against
  the new call site).
- Integration/manual: with `~/.claude/CLAUDE.md` present and the shared dir's
  `CLAUDE.md` deleted, launch an agent **via the UI** and confirm it does not
  see the host file's content. This is the actual repro and no automated test
  covers it today.

---

## 6. Provenance — what I verified vs. what I'm relaying

Reviewers should weight these differently.

**Personally read and verified:**
`platform.rs:79-125` (no seeding), `providers.rs:630-705` (placeholder + seeding
fn), exhaustive grep for callers of `seed_claude_md_placeholder_if_missing` /
`prepare_provider_auth_dir` (two production sites each),
`agent-model.ts:505-615` (UI path: `ensureAuthDir` → `WriteAgentConfigCommand`,
no `agent_open`), `SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md` in full, and
the live on-disk state of `~/.agentmux/shared/providers/claude/`.

**Relayed from a codebase survey, not independently verified:** exact line
numbers in `armory-view.tsx`, `armory-model.ts`, `global-brain-manager.tsx`,
`agent_handlers/memory.rs`, `agent_config.rs`, `editor_handlers.rs`; the claim
that no `db_*` provider-config table exists; and the `SPEC_ARMORY_DROP_HOST_CLI_CONFIG_BLOCK`
stale-status note. Quoted UI strings and the overall shape are consistent with
the spec chain I did read, but confirm line numbers before editing.

**Not investigated:** whether any non-Claude provider has an equivalent
user-level config file (§3.3) — this is an open question, not a gap I closed.
