# REPORT: Agent bundles — refine or drop?

**Status:** proposed — research for task #21780, nothing built.
**Date:** 2026-11-03
**Author:** Agent5 (Agent5@narko), at the operator's request

> "we need to refine the concept of bundle, or perhaps drop it entirely? earlier I thought it would be important as a transport method, but the way connectors and memory has growth, and the worse part is the huge sprawl, many bundles get created. are there better patterns for its essential goal, which was to be able to carry an agent's ID, memory, and history." (operator, 2026-11-03)

## 1. Recommendation

**Stop bundles being the thing that carries an agent, and stop minting them per agent. Do not delete the table yet.** In three steps, each its own PR:

1. **Stop the sprawl (small, safe).** Create an agent's own bundle lazily, on the first instruction or skill written to it, not at `agent.define`. Then one cleanup migration deletes the bundles no agent points at and that hold nothing (881 of 926 on this machine).
2. **Say what a bundle is for.** Today it is three things under one name: Global Memory entries, a reusable instruction set an agent can pick up, and the `.abf` file used to move an agent. Keep the first two (rename the second "instruction set" if the operator likes), and take the third out of the bundle (step 3).
3. **Carry an agent by reference, not by copy.** An agent already has a stable id; its definition, history and Personal Memory already live in the account-wide `shared/` store and follow it across installs and channels. Make "move this agent" attach to that id (same machine or account: nothing to copy; another machine: the existing LAN/WAN sync; another person: a fork that records where it came from). Keep `.abf` export/import only as the file form of a fork, for sharing a template or a widget, and say so.

Why not drop bundles at once: widgets and skills ride in the `.abf` export, two Global-Memory-style entries are bound as an agent's own "memory" bundle, and I found no measurement of how often export/import is used (Manoz has none either). Dropping is a real option (§5, option 3), but it should follow a few weeks of the lazy change plus a usage count, not precede them.

## 2. What is on this machine today (one read-only measurement)

Read-only queries of the shared store (`shared/store.db`, opened `mode=ro`) and of the per-channel object stores, 2026-11-03.

| Measure | Value |
|---|---|
| Bundles in the shared store | **926** |
| Global Memory entries (`is_global`) | 25 (6 of them system-owned) |
| Blank bundle | 1 |
| Per-agent and other | 900 |
| Bundles with any instructions | 30 (of which 25 are the Global Memory entries) |
| Bundles with MCP servers, skills or context files | **0** (MCP was removed from bundles in #4475; skills moved to reference tables, which hold 0 rows; context files are stored but never delivered) |
| Agent definitions in `shared/agents/definitions` | 56 |
| Agents in the live channel's store (0.60.13) | 41; 13 point at a bundle; 9 distinct bundles |
| Bundles referenced by any agent row in any store on this machine (live channel, its 7 older versions, the old root store, ~200 dev instances) | **28** |
| Unreferenced, non-global bundles | **887**, of which **881** are named "<Agent> — ABF" or "— ABF (n)" and hold nothing |
| When the 887 were created | 2026-09: 122, 2026-10: 763 |
| Bundle version rows | 212, of which 202 belong to unreferenced bundles; 128 are Global Memory seed writes |
| Most copies of one agent | AgentY 28, "Agent3/4/5, Lark, Pconat, Camper, Korp…" 27–28 each; "Claude" 57; two test families 28 each |

Two of the 13 live bindings are not per-agent at all: Camper and Korp use the shared "Workspace Rules" entry as their `memory_id`, and AgentX, Agent2 and two test agents use "Agent Memory". So the one field serves as both "my own bundle" and "a shared memory entry".

### What creates bundles

| Source | Code | Effect |
|---|---|---|
| New agent (spawn, define, fork) | `bundle_provision_for_new_agent`, `agent_def_provision_and_bind_bundle` (`storage/agents/definition.rs`); `agent_define.rs` | one empty "<Name> — ABF" per agent, by design (`ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md`: "every agent must have an ABF") |
| Migration m0021 | `migrations/m0021_backfill_agent_bundles.rs` | before the cross-channel sidecar (`agent_bundle_sidecar.rs`), every channel's first boot minted a **new** bundle for every agent it saw into the one shared store. This is the 763 of October |
| Seeds | `agent_seed.rs`, `operator_config_seed.rs` | the Global Memory entries and "Agent Memory" / "Workspace Rules" |
| People | the bundle view, Global Memory editor | named bundles and Global Memory entries |
| Import | `bundle_import.rs`, `import_for_agent` | a bundle from an `.abf` file |

The sidecar stopped the per-channel duplicates, but nothing deletes an agent's bundle when the agent is deleted (step 4 of the mandatory-ABF spec was written "if wanted" and never built), and test agents (`ScrollPinTest-*`, 56 bundles) and dev instances each add more.

### What a bundle actually delivers

- An agent's own bundle is read at launch, after Global Memory (Manoz, from the code). An empty one adds nothing, and 96% of them are empty.
- Skills bind to a bundle id (`app_api/skill.rs`), so a skill added to an agent attaches to its own bundle. That is the one case where the per-agent bundle gets content, and it is why a lazy create needs a get-or-create there.
- The `.abf` export/import (`bundle_export.rs`, `bundle_import.rs`, `export_for_agent`, `import_for_agent`, the import modals) is the transport. No MCP tool exposes it. The only measurement I could make of its use is that `db_bundle_versions` records **one** row with source `import`.

About 16,000 lines of Rust, 7,300 of frontend and 131 files carry "bundle" in the name; 17 docs mention bundles or ABF.

## 3. What has overtaken bundles

| The job | Done today by | Bundle needed? |
|---|---|---|
| Who the agent is (id, name, slug, provider, working directory, container, seat) | the agent definition in `shared/agents/definitions` plus the registry; the agent's own provider since v0.3 | no |
| Its history | the global transcript store and the history registry, keyed by the agent's id; fork = exact copy with history, spawn = worker plus brief | no |
| Its Personal Memory | `db_agent_native_memory` in the shared store (128 files, 16 agents here), versioned; "memory follows the agent" (`SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md`) | no |
| Workspace-wide instructions | Global Memory (still stored as bundles with `is_global`) | the table only |
| Credentials | Vault and per-user accounts; MCP servers are Connectors bound to an agent | no |
| Where it runs | seats, container images, LAN and WAN instances | no |
| Reusable instruction sets and skills | bundle, picked by an agent | **yes** |
| A file to give someone | `.abf` export | **yes**, with widgets |

Nearly all of what a bundle was meant to carry is already held by reference to the agent's id. What is left that only a bundle does is small.

## 4. Outside patterns (sources)

Gathered from primary documentation on 2026-11-03; a fetch tool summarised the pages, so check any load-bearing line against the page. Items I could not confirm are marked.

| Pattern | Copied or referenced | Duplicates and versioning | Secrets | Lesson for us |
|---|---|---|---|---|
| Dev containers ([spec](https://containers.dev/implementors/spec/)) | A small committed recipe references an image and Features; the environment is rebuilt | The repo is the one canonical copy; Features are versioned OCI artifacts | Forwarded at run time, not stored in the file | A recipe that points at things beats a copy of the things |
| OCI images and artifacts ([image manifest](https://github.com/opencontainers/image-spec/blob/main/manifest.md)) | Manifest points to blobs by digest; `subject` links related artifacts | Content addressing removes duplicate blobs; the digest is the identity, tags are mutable | Left to the producer | Identity by content works for immutable things, not for an agent's evolving memory |
| Docker volumes ([docs](https://docs.docker.com/engine/storage/volumes/)) | State lives in a named volume the container references | The name is the id; moving a volume between hosts means a tar copy, which forks it | Not covered | A copy made outside the system becomes a second identity |
| VS Code Settings Sync and Profiles ([sync](https://code.visualstudio.com/docs/configure/settings-sync), [profiles](https://code.visualstudio.com/docs/configure/profiles)) | One account-keyed cloud copy, per-category sync; machine-scoped settings are excluded | Last 20 remote versions kept, conflicts resolved by the user; a profile export is a separate shareable file | Tokens stay in the OS store | Closest to our case: one canonical copy per account, a separate file form for sharing, and an explicit "machine-only" class |
| Git as transport ([Codespaces dotfiles](https://docs.github.com/en/codespaces/setting-your-user-preferences/personalizing-github-codespaces-for-your-account), [Letta MemFS](https://docs.letta.com/concepts/memfs)) | The repo is cloned or projected | Remote is canonical, hashes give history; forks are cheap, so sprawl needs discipline | Must be ignored by hand | Good for small text such as memory and instructions, poor for transcripts |
| Claude Code ([memory](https://code.claude.com/docs/en/memory), [sessions](https://code.claude.com/docs/en/sessions)) | CLAUDE.md files go through git; auto memory and transcripts are machine-local, keyed by project path | The session lookup refuses an ambiguous hand-copied duplicate rather than guessing; branching makes a new id | No secrets handling in memory | Name the ambiguity and refuse it; machine-local by default |
| Letta Agent File `.af` ([repo](https://github.com/letta-ai/agent-file)) | A snapshot: prompt, memory blocks, tools, messages | Import creates a new agent, so each import is a fork; no format version documented | Secrets are set to `null` on export | The same design as our `.abf`, with the same duplicate problem |
| OpenAI Assistants and Threads, then Responses ([guide](https://developers.openai.com/api/docs/guides/migrate-to-responses)) | Server-held state referenced by id | One canonical store; forks branch from a response; the Assistants API was retired (the guide gives 2026-08-26; I did not open the date on the page myself) | Keys stay with the account | Reference by id works, but tying it to one vendor account has a cost: no export |
| LangGraph persistence ([docs](https://docs.langchain.com/oss/python/langgraph/persistence)) | `thread_id` plus a cross-thread store on a shared backend | Checkpoints can fork; growth is unbounded unless pruned; cross-version migration not documented (unverified) | Encrypted serialiser available | Pruning is part of the design, not an afterthought |
| A2A Agent Card ([spec](https://a2a-protocol.org/latest/specification/)) | A small public card with identity, skills, auth needs; the agent is referenced by URL | One canonical card, version fields | Declares schemes, holds none | Identity is a small document; state is out of scope |
| Local-first and sync ([Ink & Switch](https://www.inkandswitch.com/essay/local-first/), [Replicache](https://doc.replicache.dev/concepts/how-it-works), [Syncthing](https://docs.syncthing.net/users/syncing.html)) | Nothing is copied; peers exchange changes against stable ids | Replicache has a canonical server; Syncthing renames the older of two conflicting files | Keys stay on each device | "Identity by reference, data by sync" is the pattern our shared store already follows |
| MCP config and MCPB bundles ([MCPB](https://github.com/modelcontextprotocol/mcpb)) | `.mcp.json` references; an `.mcpb` zips a server with its manifest | Manifest has a version and platform constraints | Sensitive fields injected at run time, kept out of the file | The industry already uses "bundle" for a zip of code; ours means something else |

Evidence on export/import going wrong comes from public issue trackers rather than design guides (secondary): an import that regenerates ids so a re-import on the source machine doesn't collide ([shell-agent-v2](https://github.com/nlink-jp/shell-agent-v2/releases/tag/v0.4.0)), and an export taken while the agent kept writing, which duplicated logs in the restored copy ([elizaOS #33498](https://github.com/elizaOS/eliza/issues/33498)).

**Six principles that recur.**
1. One stable id and one canonical store; anything else is a recipe or a snapshot that points at it.
2. Reference what is immutable; sync what changes; do not copy it.
3. An export that makes a new id on import is a fork by design. If continuity is wanted, import must attach to the existing id or record where it came from.
4. Keep secrets, device-local state and machine-scoped settings out of the artifact.
5. Version the schema, and refuse an ambiguous duplicate instead of picking one.
6. Treat transcripts and memory as machine-local unless a sync layer carries them.

AgentMux already follows 1, 2, 4 and 6 for history and memory. The bundle's `.abf` is the one part built the other way: a copy that, on import, becomes a new thing.

## 5. Options

### Option 1 — Stop the sprawl, keep bundles as they are
- **Does:** lazy own-bundle (created on first instruction or skill write); one migration deleting unreferenced, empty "— ABF" bundles and their version rows; delete an agent's own bundle with the agent.
- **Removes:** about 880 rows and 200 version rows here; the "(n)" suffix logic's reason to exist.
- **Cost:** small. `own_bundle_id` and `setagentbundles`/skill bind need a get-or-create; `agent_handlers/mod.rs:1011` asserts a non-empty `memory_id` for new definitions; m0021, m0030 and m0035 stay as history (Manoz).
- **Breaks:** nothing a person sees, if the get-or-create is right. Does not answer "what is a bundle".

### Option 2 — Bundle becomes a reusable instruction set; agents move by reference (recommended)
- **Does:** Option 1, then (a) rename and narrow the concept to *instruction set* (instructions, skills, Global Memory entries scoped to it); (b) Global Memory gets its own table or a clear `kind`, so it stops being "a bundle with a flag"; (c) "move/clone an agent" uses the agent id: same account needs no copy, another instance uses the sync that exists, another person gets a fork with its parent recorded; (d) `.abf` stays as the file form for sharing a template or a widget, and its import offers "attach to the existing agent" when the id is already present, instead of always creating one.
- **Removes:** per-agent bundles as a concept; the second life of `memory_id` as "my bundle"; `.abf` as the way to carry history or memory.
- **Cost:** medium. A migration for the 13 live bindings (move the shared entries to a field that says so), docs and UI wording, an import path change. Depends on the user-data sync design (not in `main` yet).
- **Breaks:** callers of `getagentbundles`'s own-bundle field; the bundle manager's per-agent view; docs and landing copy that say "every agent has a bundle".

### Option 3 — Drop bundles entirely
- **Does:** Global Memory becomes its own table (25 entries migrate); reusable instructions become agent templates (a definition that others fork, which `parent_id` already records); skills bind to agents; widgets get their own export; `.abf` and the bundle view go.
- **Removes:** about 16,000 lines of Rust, 7,300 of frontend, 131 files, 17 docs; the `armory-bundle` schemas; the whole mandatory-ABF design.
- **Cost:** large, and mostly migration and communication, not deletion: widgets ride in the export today (Manoz); skills attach to the per-agent bundle; people with `.abf` files in the wild lose the importer unless it is kept read-only.
- **Breaks:** `.abf` export and import, the bundle manager, the widget packaging path, MCP/API callers of the bundle commands.
- **Verdict:** coherent end state, and what the outside patterns point towards, but not first. Do Option 2 and count real export and import use for a few weeks; if it is near zero, Option 3 is a deletion PR, not a migration.

## 6. Open questions for the operator

1. Is moving an agent **by file** (`.abf`) something you or others do, or only by fork and the shared store? One real example would settle Option 2 versus 3 (the store records one import in total).
2. Are widgets meant to travel inside agent bundles, or should they have their own package? (`#4650` put them in the export.)
3. Should "Agent Memory" and "Workspace Rules" stay shared entries an agent picks up, or become part of Global Memory?

## Notes and caveats

- Counts are from one machine and its live channel; other machines will differ. The 887 unreferenced figure checks every agent row in every store I could find on this machine; an agent that exists only on another machine and binds to one of these bundles would not be counted.
- I did not run a test matrix or change anything. Manoz answered the intent questions from the code (they did not write the export/import; Camper, Agent3 and Lark did, and AgentY designed m0021).
- The sync layer spec named in the task (`SPEC_USER_DATA_SYNC_LAYER_2026_11_01.md`) is not in `main`; I relied on `SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md` and `SPEC_AGENT_GLOBAL_PORTABILITY_2026-06-16.md` for the sync direction.
- Outside-pattern claims marked unverified come from the research pass and were not opened on the page by me.

Agent: Agent5@narko
