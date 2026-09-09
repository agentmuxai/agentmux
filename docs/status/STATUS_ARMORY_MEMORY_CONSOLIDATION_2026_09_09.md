# Status: Armory Memory — Consolidated State and Naming Problem

**Status:** open — four unresolved items (§4–§7), three of which need a product decision before more code lands
**Date:** 2026-09-09
**Verified against:** `b65df28c2` (code, not spec prose)
**Tracking issue:** #2024 (consolidated there 2026-09-09)

---

## 1. Why this document exists

Issue #2024 was last updated 2026-07-14 and predates roughly six weeks of shipped
work. In that window the area has had commits landing almost daily, and the
terminology used to describe it has drifted far enough from the code that
discussing it accurately had become difficult.

Two corrections to how this system was being described, both established by
reading the code rather than the specs:

1. It is **not** "two separate systems." Curated content and agent-written memory
   are both **components of ABF** — `components.instructions` and
   `components.memory` respectively (ABF v0.2 §2.3).
2. Spec `Status:` fields in this area are **not trustworthy**. Nearly every
   Armory/memory spec merged since 2026-08-30 still reads `Status: Proposed`
   despite its PR having merged days later. Check `git log -- <spec>` instead.

## 2. What exists today

Three tiers, all surfaced under Armory → **Memory** (single tab with sub-nav
since #2844, after having been split into two tabs in #2734):

| Tier | Backing store | Author | Scope | How it reaches the model |
|---|---|---|---|---|
| System | `db_bundles` (system rows) | AgentMux itself | all agents | composed at spawn |
| "Global Memory" | `db_bundles` (`is_global=1`) | the operator | all agents | AgentMux composes into `.claude/AGENTMUX_MEMORY.md` **at spawn only** |
| "Personal Memory" | `db_agent_native_memory` + CLI memory dir | the agent, via `MemoryWrite` | one agent | the CLI reads it **natively and live**; AgentMux never injects it |

The two operator-facing tiers differ in a way that is easy to miss and that users
will feel: **operator content is frozen at spawn; agent memory is live.**

### Shipped since #2024 was last updated

Mandatory ABF (#2587), native-memory durable sync (#2459), version-controlled
native memory (#2674) + retention/GC (#2728), Global Memory system tier (#2782),
Global/Personal rename (#2734) then re-merge (#2844), blank-workdir fixes
(#2901, #2926), per-agent-block browsing (#2917), filter/sort (#2929), reactive
updates (#2932), tile-grid file picker (#3000), ABF v0.2 provider-override
authoring UI (#3063).

## 3. Items 2 and 3 of #2024 are resolved

- **Item 3 (Bundle-as-container)** — shipped via
  `SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md` / #2639, which added bundle-level
  MCP + Skill references. Bundles are no longer "just instructions + context
  files."
- **Item 2 (Brain/Bundle tab decision)** — overtaken by events. The tabs were
  split (#2734) and then re-merged into one Memory tab (#2844). The product call
  happened implicitly; the item's "Brain tab" framing no longer matches the UI.

## 4. Open item A — `memory` is overloaded four ways

The word currently denotes all of:

1. the Rust `Memory` struct — which is actually a **Bundle**
   (`bundle_export.rs:7`: *"table/UI say 'Bundles', the type name predates the
   rename"*);
2. `db_bundles` rows, shown in the UI as **"Global Memory"**;
3. `db_agent_native_memory`, shown in the UI as **"Personal Memory"**;
4. the `bundle_memory_*` / `deletememory` RPC surface, which operates on
   **bundles**, not memory.

**ABF's own component names already draw the correct line** —
`components.instructions` vs `components.memory`. The UI drifted from the format,
not the other way round.

### Proposed resolution (needs a decision, not yet actioned)

Align the UI to ABF rather than inventing new vocabulary:

- "Global Memory" → **Instructions** (it *is* instructions + context files)
- "Personal Memory" → **Memory** (this is the only thing that is genuinely memory)
- Rust `Memory` struct → **`Bundle`**

**Keep the split.** The distinction is real — different author, and different
update semantics (frozen at spawn vs. live). Collapsing the names would hide a
behavioural difference users will hit. What is wrong is not that there are two
things; it is that both are called "memory."

**Rejected alternative: "Shared Memory"** for the global tier. It pairs better
with "Personal" than "Global" does (ownership axis vs. reach axis), but `shared`
already means something else here — `~/.agentmux/shared/providers/…` is
host-level shared provider config — and it leaves the deeper problem (calling
operator-authored instructions "memory") untouched.

A smaller variant, if the full rename is unwanted: keep "Global/Personal" in the
UI and rename only the Rust `Memory` struct → `Bundle`. That removes the worst of
the ambiguity for maintainers without touching user-visible language.

## 5. Open item B — ABF cannot round-trip memory

`components.memory` is defined in ABF v0.2 §2.3, and the **import** side honours
it via `bundle.import_for_agent` (plain `bundle.import` ignores it, emitting the
`MEMORY_COMPONENT_IGNORED_WARNING` constant).

But **`bundle_export.rs` never emits `components.memory`** — there are zero
references to it in that file; export produces only `instructions` and `skills`.

So an agent's memory can be **imported but never exported**. For a format whose
stated purpose is portability, that is a real gap, and it is not tracked anywhere
else.

## 6. Open item C — curated memory is structurally empty (highest user impact)

Per `SPEC_MEMORY_CARRYOVER_LOAD_AND_MANAGE_2026_09_05.md`, measured live
2026-09-06:

- `select count(*) from db_bundles where is_global=1` returns **0**.
- The composed `.claude/AGENTMUX_MEMORY.md` is **byte-identical across agents**
  (1089 bytes) with **no `# Memory` section at all**.

The curated tier runs correctly and injects nothing. Nothing in the UI surfaces
that emptiness, so it is indistinguishable from working. That spec calls fixing
this the highest-value item in the area, and it is unimplemented.

Two related threads from the same spec:

- **Mid-session propagation** — editing Armory memory never reaches a running
  agent (System A is written at spawn only). Open since
  `TOKEN_TAX_ANALYSIS_2026_06_19.md`; explicitly out of scope there.
- **Post-compaction digest** — originally requested by the repo owner, but the
  spec's own measurements show memory *already* survives compaction, so it
  recommends dropping the idea. Needs an explicit decision to close.

## 7. Open item D — north-star vs. actual direction

`ARCHITECTURE_ARMORY_FOUNDATION_CONSOLIDATION_2026_08_19.md` §3.4 wants the
`listmemories` / `getmemory` / `upsertmemory` / `deletememory` /
`reorderglobalbrain` surface **retired** in favour of `bundle.*`.

PR #3115 (2026-09-08) did the opposite: it gave that surface generated TypeScript
types and test coverage as part of the `register_typed` migration. That is
reasonable work on its own terms, but it entrenches the surface the north-star
document wants removed.

§3.1–§3.6 of that document are otherwise unstarted as a program. Worth an
explicit call on whether it is still the target architecture before more work
lands in either direction. (The `Korp@claudius` agent-consolidation series —
#3075/#3080/#3088/#3092/#3096, folding `db_agent_definitions`/`db_agent_instances`
into `db_agents` — may be informal progress on §3.3, but that coordination is not
documented anywhere.)

## 8. Decisions needed

1. **Naming** (§4) — full ABF alignment, the struct-rename-only variant, or leave as is.
2. **Post-compaction digest** (§6) — build it, or accept the measurement and drop it.
3. **North-star** (§7) — still the target, or superseded?

§5 and §6's scaffold/visibility work do not need a decision, only prioritisation.
Of the four, **§6 is the only one with user-facing impact today**; the rest are
naming and architecture hygiene.
