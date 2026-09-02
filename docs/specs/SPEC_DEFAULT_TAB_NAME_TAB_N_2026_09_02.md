# Spec: default tab names — "Tab N", not "tabN"

**Date:** 2026-09-02
**Status:** Proposed
**Motivated by:** direct request — *"change the default name for the tabs at
the top from tab1, tab2, to Tab 1, Tab 2, etc."*

## Background

`SPEC_TAB_GAPS_AND_NAMING_2026_04_25.md` deliberately unified two prior,
inconsistent default names (`Untitled1`, `T1`) onto plain `tab1`/`tab2`/…
That was itself a considered choice at the time, not an oversight — this spec
is a **revision** of that decision (explicit operator request), not a
correction of a bug.

## Change

All four sites that auto-generate a default tab name now produce `"Tab {N}"`
(capital T, space, number) instead of `"tab{N}"`:

- `agentmux-srv/src/backend/wcore/tab.rs` — `create_tab` (the "+" button path)
- `agentmux-srv/src/backend/wcore/window.rs` — new-window tab bootstrap
- `agentmux-srv/src/reducer/tab.rs` — `CreateTab` reducer (TearOffBlock /
  fresh-workspace paths)
- `agentmux-srv/src/server/service/tab_lifecycle.rs` — `handle_create_tab`
  RPC (both the computed-count branch and the `"tab1"` degenerate fallback)

These four sites already existed as separate, independently-maintained copies
of the same one-line format string before this change (a duplication noted
but not addressed here — same shape as the memory-dir-resolution duplication
fixed in `SPEC_MEMORY_RPC_HANDLERS_BLANK_WORKDIR_2026_09_02.md`, flagged as a
candidate for a shared helper if a fifth copy ever needs to change in sync
with these four again).

## Non-goals

- **No migration of existing tabs.** A tab already named `tab1`/`tab2` (or the
  even older `Untitled1`/`T1`) keeps its name; this only changes what a
  *newly created* tab (empty name passed to `CreateTab`) gets assigned. Same
  policy the original spec used for its own predecessor names.
- **No change to user-renamed tabs.** The `tab_name.is_empty()` /
  `name.is_empty()` branch is untouched — an explicit name always passes
  through verbatim.
- **No frontend change.** Naming is entirely backend-side (Rust); no
  `defaultTabName()`-equivalent exists in the frontend to update.

## Tests

Five existing assertions across four files updated to expect `"Tab 1"` /
`"Tab 2"` instead of `"tab1"`/`"tab2"`:

- `agentmux-srv/src/backend/storage/store/tests.rs`
- `agentmux-srv/src/backend/wcore/mod.rs`
- `agentmux-srv/src/reducer/tab.rs` (two assertions: first and second
  auto-generated tab in a fresh workspace)

Full `cargo test -p agentmux-srv --bin agentmux-srv` (2920 tests) run clean
after the change — confirms no other test/fixture depended on the literal
`tab1`/`tab2` string beyond these five (several other `"tab1"` literals in
the codebase are `tab_id`/identifier values, unrelated to the display name,
and were left untouched).
