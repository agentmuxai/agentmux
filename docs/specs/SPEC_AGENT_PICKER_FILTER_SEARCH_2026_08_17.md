# SPEC: Filter/search box atop the AgentPicker

**Date:** 2026-08-17
**Author:** AgentY
**Status:** Design analysis — needs answers to the two open questions below before implementation.

---

## Problem

The AgentPicker (`frontend/app/view/agent/components/AgentPicker.tsx`) has two tiers:
**My Agents** (`MyAgentsList.tsx`, up to 20 rows by default, backend-capped at 100) and
**+ New from template** (the seeded-template card grid). Both are plain scrollable
lists — finding one specific agent by eye gets slow as the My Agents list grows.
There's no way to narrow either list by typed text today; the only existing narrowing
is `MyAgentsList`'s `identityId` filter (account-scoped, not name-scoped) and the
Hidden Templates collapse.

Goal: a single-line filter input, visible at the top of the picker pane, that narrows
**My Agents** (the explicit ask — "quickly filter the existing agent we are interested
in") by substring match against the agent's name as the user types.

---

## Current state of the code

- `AgentPicker.tsx:785-882` renders, in order inside `.agent-picker`: `MyAgentsList`,
  the `+ New from template` header/hint, the template card grid (`templates()` — a
  `createMemo` filtering `agents()` to `is_seeded === 1`), `HiddenTemplatesSection`,
  then the Node.js-missing notice.
- `MyAgentsList.tsx` fetches via `ListRecentSessionsCommand` (`RecentSessionRow[]`),
  keyed as a Solid resource on `filterId` (derived from the optional `identityId`
  prop). `limit` defaults to 20; the backend hard-caps at 100
  (`CommandListRecentSessionsData`, `agentmux-srv/src/backend/rpc_types/instance.rs:104-121`).
  There is no name/text filter parameter on that RPC today — `identity_id` is the only
  server-side filter.
- `.agent-picker` already carries a top padding offset for the floating
  `PaneTabStrip` (`frontend/app/view/agent/styles/_picker.scss`, shipped in PR #2618)
  — the filter bar sits below that offset, same as everything else in the pane.
- A component named `AgentSearchBar.tsx` already exists in this same directory but is
  unrelated: it's the in-session **transcript** search overlay (Ctrl+F, searches
  loaded document nodes within one agent's conversation). Naming the new component
  something distinct (see below) avoids confusion with that one.

---

## Target design

### Placement

```
┌──────────────────────────────────────────┐
│  🔍 Filter agents...                  ✕  │  ← new: AgentPickerFilterBar
│                                            │
│  My Agents                            (3) │
│  ┌────────────────────────────────────┐  │
│  │ 🤖 Maks                            │  │
│  │    ...                             │  │
│  └────────────────────────────────────┘  │
│                                            │
│  + New from template                      │
│  ┌────────────────────────────────────┐  │
│  │ Claude Code   Codex CLI   Cursor   │  │
│  └────────────────────────────────────┘  │
└──────────────────────────────────────────┘
```

One shared input at the very top of `.agent-picker`, above `MyAgentsList`. Reuses the
existing `Input`/`InputGroup`/`InputLeftElement` primitives from `@/element/input`
(already used elsewhere for a left-icon text field) rather than the floating-UI
`search.tsx` overlay (that one is a positioned popover keyed to a terminal anchor —
wrong shape for a static inline field) or `AgentSearchBar.tsx` (wrong domain, see
above).

### Data flow — client-side filter, no backend RPC change for v1

`AgentPicker.tsx` owns one `createSignal<string>("")` (`filterQuery`), passed down as
an `Accessor<string>` to both consumers:

- **`MyAgentsList`**: new optional prop `nameFilter?: Accessor<string>`. A
  `createMemo` derives the rendered list from `rows()` filtered by case-insensitive
  substring match against `instance_name || definition_name`. Matches
  `MyAgentsListProps`' existing `identityId?: Accessor<...>` pattern (optional,
  reactive, defaults to "no filter" when absent/empty) so the component stays
  independently testable and usable without a filter bar in other contexts.
- **`AgentPicker.tsx`**: the existing `templates()` memo gains the same
  case-insensitive substring check against `agent.name` (see Open Question 1 — whether
  this is even wanted).

No new RPC. The existing `ListRecentSessionsCommand` limit/cap (20 default / 100 hard
cap) already returns more than almost any real user's agent count; filtering the
already-fetched page client-side is sufficient for v1 and avoids a schema/index change
for a `LIKE`-style backend search.

**Coverage past the default page:** `MyAgentsList`'s resource is keyed on `filterId`
(identity). Add a second key component — whether `nameFilter()` is non-empty — so that
the moment the user types the first character, the resource re-fetches with
`limit = 100` (the backend's own cap) instead of the default 20, guaranteeing the
filter searches the full available set rather than silently missing an agent outside
the first page. Reverts to the caller-provided `limit` (default 20) when the filter is
cleared, so the common no-filter case keeps today's lighter fetch.

### Empty / match-count states

- Non-empty query, zero matches: new distinct copy (mirroring the existing
  `EMPTY_GLOBAL` / `EMPTY_FILTERED` / `FETCH_ERROR` constants pattern in
  `MyAgentsList.tsx`) — e.g. `No agents match "{query}".` — kept distinct from
  `EMPTY_FILTERED` (identity-filter-empty) so tests and future debugging can tell
  which kind of "empty" is showing.
- Match count: the existing `agent-recent-sessions-count` badge already shows
  `rows().length`; once filtering is wired in it naturally reflects the filtered
  count instead — no separate counter needed.

### Interaction

- Plain `<input>`, `oninput` updates `filterQuery` directly — no debounce needed;
  filtering is a cheap in-memory substring check over ≤100 rows, and the
  limit-bump refetch (above) only fires once per empty→non-empty transition, not per
  keystroke.
- A trailing "✕" clear button appears when the field is non-empty (same visual
  language as `AgentSearchBar`'s close button, different behavior: clears text, does
  not hide the bar — the bar is always visible, unlike the Ctrl+F overlay).
- `Escape` while focused clears the field (does not close/hide the bar).

---

## Open questions (need an answer before implementation)

**Q1: Does the same query filter the template grid too, or My Agents only?**

The user's ask was specifically "filter the *existing agent* we are interested in" —
that's My Agents. Filtering templates with the same box is cheap to add (same memo
shape) and arguably harmless, but:
- Templates are a short, fairly static list (one card per installed harness) —
  scanning it by eye is not the problem being solved.
- A shared query risks a confusing state where a My Agents match exists but the
  visually-adjacent template grid also silently shrinks for an unrelated reason.

Recommendation: My Agents only for v1 (simpler, matches the literal ask); revisit if
the template grid grows enough to need it. Flagging rather than deciding, since it's
a two-line change either way and easy to get wrong for the wrong reason.

**Q2: Should the filter input autofocus when the picker pane opens?**

`AgentCard` already sets `defaultFocus={index() === 0}` on the first template card for
keyboard nav. Autofocusing the filter input would compete with that. Recommendation:
no autofocus — the user opens the pane primarily to click something they can already
see most of the time; typing to filter is the secondary path and can be reached with a
single click/Tab. Flagging in case the intended usage is closer to "always type first."

---

## Implementation plan

1. **`MyAgentsList.tsx`**: add `nameFilter?: Accessor<string>` prop; derive a filtered
   memo from `rows()`; bump the resource key/limit when filtering (per "Coverage past
   the default page" above); add the zero-match empty-state constant + branch.
2. **New `AgentPickerFilterBar.tsx`** (co-located with `AgentPicker.tsx`'s other
   sub-components): input + clear button, `value`/`onInput`/`onClear` props — plain
   presentational component, no data fetching of its own (mirrors `AgentSearchBar`'s
   prop shape without inheriting its Ctrl+F/overlay behavior).
3. **`AgentPicker.tsx`**: own the `filterQuery` signal; render
   `AgentPickerFilterBar` above `MyAgentsList`; thread `nameFilter={filterQuery}` into
   `MyAgentsList`; apply Q1's answer to the `templates()` memo.
4. **`_picker.scss`**: new `.agent-picker-filter-bar` rule block, spacing consistent
   with the existing `.agent-picker` padding-top convention from PR #2618.
5. Tests (per repo convention — tests ride with the code, no doc-only PRs):
   - `MyAgentsList.test.tsx`: substring match narrows rows (case-insensitive); clearing
     restores the full list; zero-match empty state renders the new distinct copy;
     typing triggers a refetch with the bumped limit.
   - `AgentPicker.test.tsx`: filter bar renders above `MyAgentsList`; typing narrows
     what `MyAgentsList` receives; clearing restores it; (template-grid behavior once
     Q1 is answered).
6. Changeset (`type: minor` — new user-facing capability, following the harness/model
   creation-flow changeset's precedent for similarly-scoped picker UX additions).

Estimated size: ~150–250 lines including tests — comparable to the smaller items in
issue #2594's delivery plan, not a large PR.

## Out of scope for v1

- Backend full-text/`LIKE` search RPC — the 100-row client cap already covers
  realistic per-user agent counts; revisit only if real usage shows people
  regularly exceeding it.
- Fuzzy/typo-tolerant matching — plain case-insensitive substring only.
- Filtering inside the collapsed `HiddenTemplatesSection` — out of scope unless
  Q1 lands as "yes, filter templates too" and hidden-template search is separately
  requested.
