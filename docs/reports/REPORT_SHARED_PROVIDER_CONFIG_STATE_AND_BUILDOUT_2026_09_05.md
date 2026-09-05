# Report: Shared provider config — current state, how agents read it, and buildout

**Date:** 2026-09-05
**Status:** Draft — assessment + proposal. Nothing here is implemented.
**Revised 2026-09-05 after Codex review of PR #2991:** the first version of this
report claimed a verified isolation defect on "the UI launch path." That was
**over-scoped and wrong for the normal launch flow** — see §3.1, which now
records the corrected, much narrower open question and what disproved the
original claim.
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
vendor CLI's own discovery. That's a legitimate design, but it makes *presence
of the file* a load-bearing isolation control: `REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md`
§2 established **by direct three-arm experiment** that an isolated
`CLAUDE_CONFIG_DIR` containing no `CLAUDE.md` reaches past itself to the
operator's personal `~/.claude/CLAUDE.md`, and that seeding any file at that
path stops it.

The two spawn paths that matter both seed correctly (§2). §3.1 records a
narrower, **unresolved** question about a third route.

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
Global** as a **single** read-only block
(`frontend/app/view/brain/global-brain-manager.tsx:163-192`):

- Section heading: *"Claude Code provider config — reference only, not part of
  Global Memory."*
- Badge: `"Claude Code — shared provider config"`; caption *"Used by default
  spawned agents."*; tooltip notes identity-bound agents use a different dir.

Path + `<pre>` preview. No textarea, no save, no edit affordance of any kind —
read-only by construction, per `SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md`
§0/§2.3.

> **Corrected after review:** an earlier draft described *two* blocks (adding a
> `"Claude Code — host CLI config"` block for `~/.claude/CLAUDE.md`). That block
> was **removed 2026-09-01** (`SPEC_ARMORY_DROP_HOST_CLI_CONFIG_BLOCK_2026_09_01.md`,
> rationale at `global-brain-manager.tsx:151-158`): once
> `REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md` proved a spawned
> agent never reads the host file, showing it invited the misreading that
> editing it would change agent behaviour. The two-block description came from
> §6/§7 of the 08-24 spec, which predates the removal — a good illustration of
> why that spec chain (three self-corrections deep) should not be read as
> current state without checking the code.

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

### 2.2 UI "Launch" (`agent-model.ts`) — dir set at launch, **re-resolved at first turn**

1. `getApi().ensureAuthDir(provider.id)` → CEF `ensure_auth_dir`
   (`agent-model.ts:519-522`). This **only `create_dir_all`s** — it does not
   seed (`agentmux-cef/src/commands/platform.rs:113-120`).
2. `envVars[provider.authConfigDirEnvVar] = authDir` — at this point the env
   points at a possibly-unseeded shared dir.
3. `RpcApi.WriteAgentConfigCommand(...)`, block meta, controller creation.
4. **Then, on the first turn**, `agent_handlers/input.rs:337` calls
   `inject_identity_env_async` **before the CLI is spawned**. For an agent with
   an active `AgentInstance` and OAuth bindings, that reaches
   `identity/resolver/inject.rs:617-667`, which:
   - blocks the spawn outright if the bound dir is the provider's ambient home,
   - **seeds the placeholder** (fail-closed — spawn is refused on error),
   - and **overwrites `CLAUDE_CONFIG_DIR`** with the per-identity dir
     (`inject.rs:667`), discarding the shared dir set in step 2 entirely.

So for the normal launch-modal flow — where identity is required at submit time
(`SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md`) — step 4 both replaces and
seeds the directory, and step 1's missing seed never matters.

`seed_claude_md_placeholder_if_missing` has exactly two production callers —
`agent_open.rs:360` and `inject.rs:648` — but between them they cover both
spawn paths above.

### 2.3 The uncovered route

`inject_identity_env_with_broker` returns early, leaving env untouched, when the
block has **no `AgentInstance` row** (`inject.rs:361-368`). The code comment
names the case: *"quick-launch panes that never went through the launch modal
are outside the managed-credentials contract."* For such a pane nothing seeds
and nothing overwrites — see §3.1.

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

### 3.1 🟡 Open question — is any spawn route left with an unseeded dir?

**Superseded claim.** The first version of this report asserted a verified
isolation defect on "the UI launch path," reasoning that
`agentmux-cef/src/commands/platform.rs:113-120` (`ensure_auth_dir`) only
`create_dir_all`s and that the UI path therefore never seeds. **That conclusion
was wrong**, caught by Codex on PR #2991 and confirmed by re-tracing: the trace
stopped at controller creation and missed the first-turn spawn step. For a
launch-modal agent, `input.rs:337` → `inject.rs:648` seeds the bound account's
dir *and* overwrites `CLAUDE_CONFIG_DIR` before the CLI ever starts (§2.2). The
normal flow is safe, and the "fresh install + UI launch" repro I described does
not reproduce.

**What is still true, and unresolved:**

1. `ensure_auth_dir` genuinely does not seed. Verified.
2. `inject_identity_env_with_broker` genuinely returns early — env untouched,
   nothing seeded, nothing overwritten — for a block with no `AgentInstance`
   row, explicitly described in-code as *"quick-launch panes that never went
   through the launch modal"* (`inject.rs:361-368`). Verified.
3. The underlying mechanism is real and **measured, not inferred**:
   `REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md` §2 shows an
   isolated dir lacking `CLAUDE.md` returns the host file's heading, and that
   seeding stops it.

**The gap in my own analysis:** I did not verify whether a quick-launch pane
actually sets `CLAUDE_CONFIG_DIR` at all. Both outcomes are plausible and they
have different implications:

- If it **does** set it (to the shared dir, via the same `ensureAuthDir` call):
  an unseeded shared dir on that route would leak, and (2)+(3) make that a real
  hole.
- If it **does not** set it: the CLI runs fully un-isolated against `~/.claude`
  anyway, which is a *known and accepted* property of quick-launch panes being
  "outside the managed-credentials contract" — not a new defect, and not
  something seeding would fix.

**Next step to resolve it** — cheap and decisive, roughly the §5 experiment
scoped to this route: open a quick-launch pane (no launch modal, so no
`AgentInstance`), dump its resolved `CLAUDE_CONFIG_DIR`, and if it points at the
shared dir, delete that dir's `CLAUDE.md`, plant a sentinel in
`~/.claude/CLAUDE.md`, and ask the agent to quote its user-level instructions —
the calibrated arm-3 prompt from the 09-01 report, which that report warns is
mandatory (an uncalibrated null reads as "no leak" and is wrong).

**If it turns out to be real,** the fix is still "make seeding a property of
provisioning the dir, not of one spawn path" — two independently-maintained
provisioning paths (`platform.rs` in the CEF host, `prepare_provider_auth_dir`
in srv) is the structural issue. But that work should not start until the
experiment above says there is something to fix.

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

### Step 0 — Settle §3.1 first (one experiment, not a code change)

Run the quick-launch-pane experiment in §3.1 before touching provisioning code.
It is cheap and it decides whether Step 1 exists at all. **Revised down from
"Step 1 — do this regardless" after review showed the normal launch flow is
already covered** — shipping a provisioning refactor on the strength of the
retracted claim would have been change without a demonstrated defect.

### Step 1 — (conditional) unify auth-dir provisioning

Only if Step 0 confirms a route that reaches a CLI with an unseeded dir. Then:
one provisioning path, seeding included, plus a regression test that exercises
*the route* rather than the functions directly — the current tests cover
`seed_claude_md_placeholder_if_missing` and `prepare_provider_auth_dir` in
isolation, which is exactly why a route that bypasses both would go unnoticed.

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

## 5. Verification

**For Step 0 (the deciding experiment):** follow
`REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md` §1's three-arm
method against a **quick-launch pane** specifically. Keep arm 3 (the sentinel):
that report records that its own first pass ran arms 1–2 only, got a null from
every arm, and would have concluded "no leak, fix unnecessary" — wrongly,
because memory files arrive as a user-turn reminder rather than literal system
instructions. Use its calibrated prompt (*"quote the first markdown heading of
any user-level instructions in your context"*), not a yes/no question.

**For Step 1, if it happens:**
- Rust unit: provisioning on a temp `HOME` leaves a `CLAUDE.md` containing the
  placeholder.
- Rust unit: an existing non-placeholder `CLAUDE.md` is **never** overwritten
  (guaranteed today by `path.exists()` at `providers.rs:697`; pin it against any
  new call site).
- Integration: exercise the *route*, not the function — the whole point is that
  route-level coverage is what's missing.

---

## 6. Provenance — what I verified vs. what I'm relaying

Reviewers should weight these differently.

**Personally read and verified:**
`platform.rs:79-125` (no seeding), `providers.rs:630-705` (placeholder + seeding
fn), exhaustive grep for callers of `seed_claude_md_placeholder_if_missing` /
`prepare_provider_auth_dir` (two production sites each),
`agent-model.ts:505-615` (UI launch flow), `input.rs:320-375` (first-turn
identity injection), `inject.rs:358-420` (early-return conditions) and
`inject.rs:600-670` (ambient-home block, seeding, `CLAUDE_CONFIG_DIR`
overwrite), `global-brain-manager.tsx:148-195` (single block),
`SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md` in full,
`REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md` §0-§2, and the live
on-disk state of `~/.agentmux/shared/providers/claude/`.

**Relayed from a codebase survey, not independently verified:** exact line
numbers in `armory-view.tsx`, `armory-model.ts`, `agent_handlers/memory.rs`,
`agent_config.rs`, `editor_handlers.rs`; and the claim that no `db_*`
provider-config table exists. Confirm before editing.

**Not investigated:** whether a quick-launch pane sets `CLAUDE_CONFIG_DIR` at
all (§3.1 — the open question this report now turns on), and whether any
non-Claude provider has an equivalent user-level config file (§3.3).

**Retracted:** the original §3.1 "verified latent defect on the UI launch path."
The error was stopping the trace at controller creation rather than following
the first-turn spawn, so the identity-injection step that seeds *and* replaces
the dir was missed. Recorded rather than quietly edited out, because the failure
mode is instructive: the survey and my own reading agreed with each other and
were both incomplete in the same direction, and nothing about the resulting
claim looked shaky from inside it.
