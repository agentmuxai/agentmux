# SPEC: `GlobalMemoryList` lists the system rows too, flagged and read-only

**Date:** 2026-10-02
**Status:** implemented — PR #4219.
**Author:** AgentX (narko), at the owner's request
**Affects:** `crates/srv/src/server/app_api/global_memory.rs` (`global_memory_list_impl`), `crates/srv/src/server/app_api/tests/global_memory_impl_tests.rs`, `crates/mcp/src/tool_schemas.rs` (the tool text)
**Builds on:** `docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md` (the system tier and why it is write-protected)

## 1. What the owner saw

The owner's Armory Global Memory pane showed a full list. An agent calling `GlobalMemoryList` got `{"entries": []}`, and the owner called it a bug.

The live store for that instance held six Global Memory rows: the five "AgentMux Operator Config" system rows (App API, Environment & Gotchas, Rich output in the agent pane, Your workspace, Swarm broadcasts) and one ordinary row that was added a few seconds after the empty answer. The pane lists all six (`global-bundle-model.ts` builds its sections from every `is_global` row and splits them into system and ordinary for display). The tool listed only the ordinary one, because `global_memory_list_impl` filtered `!is_system`, so before the ordinary row existed it returned nothing.

So the code did what it said. The result was still wrong for a reader: an empty list reads as "there is no Global Memory", when the truth was "there is Global Memory, and none of it is yours to edit".

## 2. Decision

List the system rows, flagged, and keep every mutation path closed.

- `GlobalMemoryList` returns each entry as `{ id, name, updated_at, system }`. System rows come first, as `bundle_list_global` already orders them.
- `GlobalMemoryRead` still refuses a system id. `GlobalMemoryWrite` and `GlobalMemoryRemove` still refuse one. The protections in the system-tier spec are about mutation, and all of them stay.
- No content is returned by the list, system or not.

### 2.1 Why this is safe

The system rows' content is already in every agent's context: it is composed into the startup instructions under "# [AgentMux System]". Their names and ids reveal nothing the agent has not been given. What the filter protected against was an agent being offered a handle to edit them, and the handle stays refused at read, write and remove (each checks `is_system` itself, which the new test exercises).

### 2.2 What changes for an agent

An agent that lists and then loops over the entries to update them will now meet system rows. The tool text says so (`system=true` rows are listed so you can see what is in your context, and cannot be read, changed or removed), and the refusal messages are the same ones as before.

## 3. Not changed

- The Armory pane. It already shows both tiers.
- `GlobalMemoryHistory`, `Diff` and `Revert`. They take an id, and still refuse a system one.
- The startup composition of Global Memory.

## 4. Tests

`list_includes_system_entries_flagged_and_first` replaces `list_excludes_system_entries`: with one ordinary and one system row, the list has both, system first and flagged, ordinary second and unflagged, no `content` or `instructions` key, and read, remove and write of the system id each return an error.
