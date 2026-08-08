# Report: Armory/Stash as source of truth vs. native memory — status and history

**Date:** 2026-08-07
**Author:** Agent2
**Status:** Research/status doc — no code changes. Written to ground a design
decision, not to implement one.
**Trigger:** User's proposed premise (verbatim, for traceability):

> The source of truth for agents running in agentmux are the agent stash and
> armory (which themselves should be in sync) .. the memories written to
> local claude info needs to be in strict sync. if possible, can the memory
> folder be changed, if not, they need to be strickely replicated. lets
> collect the history in light of this premise, and write a status doc to
> file.

---

## 0. Bottom line up front

- **"Armory" and "Stash" are not two stores that need syncing with each
  other — they are two views over the same four SQLite-backed tables**
  (`db_accounts`, `db_bundles`, `db_skills`, `db_mcp_servers`). Armory = the
  catalog; Stash = one agent's bindings into that catalog. For three of the
  four entities, "Armory and Stash in sync" is already structurally
  guaranteed by construction, not something to build. See §2.
- **Native memory is not part of that system at all.** It's the Claude Code
  CLI's own autonomous scratchpad — plain markdown files on disk, written by
  Claude during a session, with zero code path connecting it to
  `db_bundles`/Armory/Stash today. See §3.
- **This is not an oversight — it was a deliberate original design
  decision**, stated explicitly in the (now-archived) spec that first
  separated these concepts: *"A Memory Bundle must never contain a fact
  Claude discovered, and a native memory file must never contain a rule a
  human intended to enforce."* Adopting the new premise means consciously
  reversing that call, not just filling in a gap. See §4.
- **On the folder-relocation question:** partially yes, but it doesn't
  matter. `CLAUDE_CONFIG_DIR` (the root) is already redirectable and already
  agent/identity-scoped by AgentMux. The sub-path under that root
  (`projects/<cwd-hash>/memory/`) is fixed by Claude Code's own convention
  and can't be pointed at an arbitrary location. But AgentMux already
  computes that exact path itself and reads/writes it directly today (that's
  how Stash's "Memory" tab works) — so no filesystem trick is needed either
  way. "Strict sync," if adopted, is an application-level job (an explicit
  reconciliation step), not a filesystem one. See §5.
- **Before extending the sync guarantee to native memory, Armory/Stash have
  three known bugs where they're already not honestly in sync with the agent's
  actual runtime config** — worth fixing first, since the new premise's first
  clause ("stash and armory should be in sync with each other") already has
  open exceptions today. See §6.

---

## 1. History, chronologically

The naming and architecture have gone through several deliberate phases,
each captured in its own spec:

| Date | Doc | What changed |
|---|---|---|
| 2026-06-19 | `docs/specs/archive/SPEC_MEMORY_IDENTITY_ARCH_2026_06_19.md` | **Origin document.** Named the "Memory" collision explicitly (config presets vs. native memory files) and the "Identity" collision (credential library vs. per-agent keychain). Proposed the rename that became "Bundles," and stated the layer-separation invariant quoted in §0/§4. |
| 2026-06-19–24 | `SPEC_TRUST_CENTER_GLOBAL_BRAIN_2026_06_19.md`, `SPEC_TRUST_CENTER_MODALS_IDENTITY_MEMORY_SEED_2026_06_19.md`, `SPEC_AGENT_PANE_MEMORY_IDENTITY_MODALS_2026_06_19.md`, `SPEC_GLOBAL_IDENTITY_MEMORY_DRONE_2026_06_24.md` | Built out the "Global Brain" (is_global bundle injection) and the native-memory placeholder modal in parallel, as two visibly distinct features from day one. |
| 2026-07-02 | `SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md`, `SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md` | Trust Center → Armory; Memory-bundle-presets → Bundles. Table renamed `db_memory_bundles` → `db_bundles`; Rust method names deliberately kept as `bundle_memory_*` (internal/external naming decoupled on purpose). |
| 2026-07-10–16 | `SPEC_ARMORY_PRELOADED_CREATIVE_MCP_CONNECTORS_2026_07_10.md`, `SPEC_ARMORY_MCP_SERVER_DEFAULT_SEED_CATALOG_2026_07_13.md`, `SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md`, **`SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13.md`**, `SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md`, `SPEC_ARMORY_ACCOUNTS_NO_MODALS_2026_07_16.md` | Phase 4/5: storage rename completed; Armory's separate "Identities" tab **removed** and per-agent data pushed entirely out of Armory (the precedent §2 below relies on); responsive layout; Accounts tab de-modalized. |
| 2026-07-20 | **`ARCHITECTURE_ARMORY_2026_07_20.md`** | Canonical reference, written once the shape stabilized. States the governing principle: *"Armory holds shared, reusable resources... What deliberately does NOT live in Armory: per-agent-instance data."* Documents all 4 live Armory entities' binding mechanisms side by side (§2 below is built from this). |
| 2026-07-23 | `docs/reports/REPORT_ARMORY_ARCHITECTURE_AND_NAMING_REVIEW_2026_07_23.md` | First to name the three-way "Memory" collision precisely (Armory Memories tab / AgentSetupModal Memories tab / Armory Bundles tab) and flag the Accounts dead-write-path bug (§6.1 below). |
| 2026-07-27 | `docs/reports/REPORT_ARMORY_STASH_NAMING_2026_07_27.md` | Renamed the per-agent modal "the agent armory" → **Stash** (this doc's naming source). Confirms Stash's MCP/Skills/Startup tabs are filtered views into the *same* shared tables Armory manages — genuinely "in sync" by construction — while Accounts and (native) Memory are the two tabs that are actually agent-scoped data, not shared-catalog views. |
| 2026-07-27–28 | `REPORT_ARMORY_SKILLS_MARKDOWN_AND_BIND_BUG_2026_07_27.md`, `REPORT_MCP_SKILLS_BIND_DOES_NOT_GATE_CONFIG_2026_07_28.md` | Found and documented the bind/unbind-doesn't-gate-runtime-config bug (§6.2 below) — Armory/Stash's UI state and the agent's actual materialized config can already disagree today. |
| 2026-08-05 | `SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md` (unrelated subsystem, but the closest existing precedent for *this* premise) | For a different pair of stores (pane scrollback vs. the model's actual context), adopted the principle *"never let the two disagree silently"* rather than forcing them to always match (acknowledged as not fully achievable) — record every divergence explicitly instead. Cited here because it's the nearest existing precedent for how this codebase has previously handled "two sources of truth that can drift," and is worth weighing against a "strict sync" framing. |

**No spec, report, or commit anywhere in this history proposes connecting
Armory/Stash to native memory.** Every document either treats native memory
as correctly out-of-scope for Armory (by the same "reusable resource vs.
per-agent data" principle that removed the Identities tab), or documents the
three-way naming collision as debt without proposing a functional merge.
This is genuinely new territory, not a plan that already exists and needs
executing.

---

## 2. Is Armory already "in sync with" Stash?

Yes, structurally, for three of the four Armory entities — because they are
implemented as one table plus a join table, not two tables:

| Entity | Armory shows | Stash shows | Same underlying rows? |
|---|---|---|---|
| Bundles | All rows in `db_bundles`, full CRUD | The one bundle picked via `memory_id`, read-only select | Yes |
| Skills | All rows in `db_skills` (catalog CRUD + global promote) | This agent's own + globally-visible skills, bind/unbind | Yes — `db_agent_skills_ref` join |
| MCP Servers | All rows in `db_mcp_servers` (catalog CRUD + global promote) | This agent's own + globally-visible servers, bind/unbind | Yes — `db_agent_mcp_ref` join |
| Accounts | All rows in `db_accounts` | This agent's provider→account assignment | **No** — see §6.1, this is the one broken case |

So "Armory and Stash should be in sync" is already true by construction for
Bundles/Skills/MCP — there's a single read/write path (`Store::bundle_memory_*`,
`skill_*`, `mcp_server_*`), and both surfaces query the same rows. The actual
open risk isn't Armory-vs-Stash drift; it's **catalog state vs. materialized
runtime config** drift — covered in §6.

---

## 3. What native memory actually is, mechanically

- **What writes it:** the Claude Code CLI itself, autonomously, during a
  session — via its own memory tool. AgentMux does not decide when or what
  gets written; it only exposes a viewer/editor (Stash's "Memory" tab,
  `AgentNativeMemoryModal` → `native_memory_handlers.rs`'s
  `memory:list/read_file/write_file` RPCs).
- **Where it lives:** `$CLAUDE_CONFIG_DIR/projects/<sanitized-cwd>/memory/`.
  The sanitization/hashing scheme (`memory_dir_for_cwd`,
  `agentmux-srv/src/server/native_memory_handlers.rs:46-66`) deliberately
  mirrors Claude Code's own `sessionStoragePortable.ts` convention — this is
  Claude's path-naming rule, replicated, not a path AgentMux invented.
  `CLAUDE_CONFIG_DIR` itself is already resolved per-agent/per-identity
  (`claude_config_dir_for_identity`, same file, lines 175-193): unbound agents
  get `~/.agentmux/shared/providers/claude`, identity-bound agents get
  `~/.agentmux/shared/identities/<id>/claude`.
- **What reads it:** Claude Code itself, at the start of every session
  (`MEMORY.md` auto-loads; topic files load on demand). Nothing in AgentMux's
  own config-generation path (`agent_config.rs`, `agent_open.rs`) reads or
  writes this directory — it is completely outside the CLAUDE.md/`.mcp.json`
  materialization pipeline that Bundles/Skills/MCP go through at every
  launch.
- **Confirmed: zero existing code path connects it to Armory/Bundles.**
  `bundle_self_get_impl` only ever touches `db_bundles`; the native-memory RPC
  handlers only ever touch the on-disk directory. The only code that
  "knows about" both is a shared agent/identity-lookup helper used for
  routing requests to the right agent — not for moving content between them.

---

## 4. The original design invariant this premise would revise

The archived origin spec (`SPEC_MEMORY_IDENTITY_ARCH_2026_06_19.md` §5.1)
states the intended relationship between these two things explicitly, in a
table titled "Layer separation invariant":

| What | Written by | Injected as |
|---|---|---|
| Rules / policies / tool configs | Human, via Bundles | `CLAUDE.md` at every launch |
| Discovered facts / patterns | Claude, autonomously | `MEMORY.md` at session start |

> **The invariant:** A Memory Bundle must never contain a fact Claude
> discovered, and a native memory file must never contain a rule a human
> intended to enforce. If that drift happens, a quarterly review should
> promote facts to bundles or prune them.

This is a considered, explicit design choice: the two are supposed to stay
**categorically separate** (rules vs. facts), reconciled only by an
occasional human review, not kept in continuous sync. The new premise —
"the memories written to local claude info need to be in strict sync [with
Armory/Stash]" — is a real proposal to change that, not a bug fix for
something that was supposed to already work this way. Worth being explicit
about which of two things is actually being asked for, since they imply very
different designs:

- **(a) Durability/portability sync** — ensure the *same* native memory
  content is reliably reachable for a given agent no matter which
  channel/instance/build it's opened from, and never silently lost, by
  treating Armory/Stash's own agent-identity records as the anchor that
  resolves *where* that agent's memory lives. This does not touch content —
  Claude still owns what's written — it just makes location resolution and
  persistence as reliable as everything else Armory/Stash already manage.
- **(b) Content sync** — treat Bundle content as authoritative and push it
  into (or reconcile it against) the native memory files, or vice versa. This
  directly reverses the invariant above and would need a real answer to "what
  happens when Claude writes a fact that contradicts a synced-in rule."

§0/§5 below assume (a), since it's what the concrete ask ("can the folder be
changed," "strictly replicated") technically describes — a location/durability
problem — but this is the single most important thing to confirm before any
implementation starts.

---

## 5. Answering "can the memory folder be changed?"

**Root: yes, already done.** `CLAUDE_CONFIG_DIR` is fully AgentMux-controlled
per agent/identity today (§3). This is the lever that already exists for
"which `~/.claude`-equivalent tree does this agent use."

**Sub-path: no, and it doesn't need to be.** The `projects/<cwd-hash>/memory/`
structure under that root is a Claude Code CLI convention, not a setting —
there's no flag or env var that decouples the memory folder from the
cwd-encoding, and nothing in this codebase's own research
(`docs/research/claude-code-presentation-layer.md`) or history proposes a
symlink/relocate workaround. But this is moot: AgentMux's own
`memory_dir_for_cwd`/`memory_dir_for_agent` already **replicates Claude's
exact algorithm**, so AgentMux can always independently compute and directly
read/write the exact folder Claude Code itself will use — that's the entire
mechanism Stash's "Memory" tab already relies on today. There is no need to
relocate anything to get direct access; direct access already exists.

**What's actually missing, if (a) from §4 is the goal**, is not filesystem
access — it's:
1. A guarantee that `memory_dir_for_agent`'s two-tier lookup (persisted
   `db_agents` instance row → global named-agent-registry fallback) always
   resolves to the *same* folder for the *same* logical agent, across
   whatever channel/instance/build it's opened in — i.e., extending the same
   "agents are global, not per-channel" guarantee CLAUDE.md already states
   for agent definitions/registry to native memory's location-resolution
   path specifically, and confirming (with a test) that it holds.
2. Optionally, a periodic or on-close capture of the on-disk memory content
   into a durable, AgentMux-owned record (so a wiped/corrupted local
   filesystem doesn't lose it) — this would be actual replication, not just
   location-consistency, and is the literal reading of "strictly replicated."

---

## 6. Existing gaps in "Armory/Stash in sync with runtime reality" (fix candidates, independent of native memory)

These predate this premise and are worth listing because the premise's first
clause — Armory and Stash should be in sync with each other and, implicitly,
with what an agent actually runs with — already has open exceptions:

### 6.1 Accounts: Stash writes a column nothing reads (bug)

`Stash → Accounts` (`AgentIdentityModalPanel`) writes provider assignments
into the legacy `AgentDefinition.accounts` JSON blob. The actual spawn-time
resolver (`identity/resolver.rs::resolve_bindings_for_instance`) reads
**only** `db_agent_identity_links`, written exclusively by the agent-launch
flow. Result: the modal looks fully functional (pick a provider, assign an
account, see it save) but has zero effect on what the agent actually launches
with. Documented in `ARCHITECTURE_ARMORY_2026_07_20.md` §1 and
`REPORT_ARMORY_ARCHITECTURE_AND_NAMING_REVIEW_2026_07_23.md` §2.2. Not yet
fixed.

### 6.2 MCP/Skills: unbind doesn't gate runtime config (bug)

`bound_to_agent` — the flag every Bind/Unbind toggle in Armory and Stash's
MCP Servers/Skills tabs is built on — has no effect on `agent_open.rs`'s
config generation, which injects **every** global MCP server/skill into
**every** agent unconditionally, regardless of bind state. Unbinding flips the
UI badge; the item keeps being injected into the agent's next generated
config regardless. Documented in
`docs/reports/REPORT_MCP_SKILLS_BIND_DOES_NOT_GATE_CONFIG_2026_07_28.md`,
mitigated today with UI copy only ("this doesn't do what it looks like yet"),
not fixed — a real fix needs a backfill migration first (§ that report's own
recommended follow-up, not repeated here).

### 6.3 Bundles: non-global binding is pull-only, not materialized

`is_global` bundles auto-inject into every agent's CLAUDE.md at launch. A
non-global bundle bound via `db_agent_instances.memory_id`, by contrast, is
only ever *fetchable on demand* (`bundle.self.get`) — nothing reads
`memory_id` at config-generation time to actually inject that bundle's
instructions into CLAUDE.md the way every other bound entity gets
materialized. Documented in `ARCHITECTURE_ARMORY_2026_07_20.md` §2. Not a
bug exactly (the architecture doc calls it "the weakest binding, but not
inert" — an agent/caller *can* pull it explicitly), but it's the one entity
whose "bound" state and "materialized" state most visibly diverge.

### 6.4 Cosmetic, lowest priority

`ArmorySection` id/label mismatch (`id:"brain"` renders as "Memories,"
`id:"memories"` renders as "Bundles") — residue of the Phase 5 label-only
rename, purely a code-readability hazard, no user-facing or behavioral
effect. Flagged in two reports, not yet fixed, explicitly low-risk/low-priority
in both.

---

## 7. Open questions before implementing anything

1. **Confirm which reading of "strict sync" (§4) is intended** — durability/
   location-consistency, or actual content reconciliation between Bundles and
   native memory. These are different-sized projects with different risk
   profiles, and the second one reverses a stated design invariant rather
   than closing a gap.
2. **If content sync is actually wanted:** what should happen when Claude's
   autonomously-written memory contradicts a human-authored bundle rule?
   The original invariant's answer was "neither automatically wins — a human
   reconciles it during periodic review." Any automated sync needs an
   explicit new answer to this, or it needs to pick one side as always
   authoritative.
3. **Should §6's three pre-existing gaps be fixed first?** They're smaller,
   already-scoped, already-diagnosed (each report above has a concrete
   recommended fix), and closing them would make "Armory and Stash are in
   sync with runtime reality" true in more cases than it is today — arguably
   a prerequisite for trusting them as "the source of truth" the new premise
   wants to lean on.
4. **If durability/location-consistency (§5) is the goal:** is there a known
   failure case today — e.g. a per-build-channel instance genuinely losing or
   failing to find an agent's native memory folder — or is this precautionary?
   Worth a quick reproduction check (open the same agent from two different
   channels/instances, compare `memory_dir_for_agent`'s resolved path) before
   scoping a fix, since §5 already shows the underlying mechanism *should*
   resolve consistently by construction — confirming whether it actually does
   in practice would sharpen whether this is a real bug or a defense-in-depth
   ask.
