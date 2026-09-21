# SPEC: Reusable fuzzy/synonym search utility, first shipped in the Settings pane

**Date:** 2026-09-21
**Status:** proposed
**Related:** `docs/specs/SPEC_SETTINGS_PANE_2026_06_25.md`, `docs/specs/SPEC_SETTINGS_PANE_COMPLETION_2026_07_14.md`,
`docs/specs/SPEC_SETTINGS_AUDIT_GOOD_PICKINGS_2026_08_19.md` (Settings pane history),
`docs/specs/command-palette.md` (existing search-UX precedent, migrated onto this utility — §4),
`docs/specs/SPEC_AGENT_PICKER_FILTER_SEARCH_2026_08_17.md` (existing filter-bar precedent,
also migrated onto this utility — §5; this spec follows its "Open questions with a
recommendation" format).

---

## 0. The ask

A search bar at the top of the Settings pane, where typing finds a setting even when
the user's word isn't the setting's literal label ("synonyms also match" — researched
against real precedent, §1). Widened during design: three call sites in this app
independently reinvent text search (command palette, agent-picker filter, and now
Settings), so build one small reusable utility and have all three use it — not just
Settings. Swarm and Toolchain are named as likely future consumers, so the utility is
designed generically rather than settings-shaped, even though this PR doesn't add
search UI to either of those yet (§7).

## 1. Research: how synonym-aware search actually works in practice

Three real precedents, all converging on the same answer:

- **Client-side fuzzy-match libraries (Fuse.js, FlexSearch, MiniSearch) do not do
  synonym matching.** They solve a different problem — typo/reordering tolerance via
  edit-distance or n-gram scoring — and every comparison of them turns up nothing about
  built-in synonym expansion. The standard advice for closing "the gap between your
  product's jargon and users' words" is a **synonym map maintained as static data**,
  applied before or alongside the fuzzy match, not a library feature.
- **macOS Spotlight's core index is explicitly lexical, not semantic** — "a lexical
  index has no concept of synonyms... searching for 'automobile' won't find files
  labeled 'car.'" The widely-believed "System Preferences search is smart" experience
  (type "wireless," find the Wi-Fi pane) is **not** semantic/ML search under the hood —
  historically it's each preference pane shipping its own small, hand-authored list of
  alternate search terms (`NSPreferencePane`'s search-keywords mechanism), which the
  lexical index then matches literally.
- **This repo's own closest precedents already do the "cheap" half of this pattern**:
  the command palette (`frontend/app/modals/command-palette.tsx:33-43`, pre-refactor)
  and the agent-picker filter (`frontend/app/view/agent/components/MyAgentsList.tsx:817-825`,
  pre-refactor) both did a hand-rolled `.toLowerCase().includes(q)` substring match —
  no fuzzy library, no synonyms. That's fine for short, predictable strings (command
  labels, agent names); it under-delivers for a ~59-item settings corpus with jargon-y
  labels ("Ligature rendering," "Idle timeout") where "synonyms match" was the explicit
  ask.

**Conclusion — the actual best practice for this UI shape (a small, fixed, well-known
corpus, searched interactively, in a local desktop app):** a **curated keyword/synonym
list per item** for the domains that need it (Settings — §3.3), authored once as
static data, matched with a **lightweight fuzzy-match layer** on top for typo/reorder
tolerance and relevance ranking, shared as one utility across every text-search UI in
the app. Not a runtime LLM call (keystroke-latency search can't afford a network or
inference round-trip per character), and not embeddings (every corpus here is far too
small and static to need it). This mirrors what Spotlight/System Preferences actually
ship, not the ML search users often assume is happening.

Package to add: **Fuse.js** (~4KB, zero runtime deps) — no fuzzy-search library exists
in this repo today (checked `package.json`; only unrelated `@codemirror/search` and
`@xterm/addon-search`), and Fuse.js is specifically positioned for "finding what the
user meant... command palettes, contact lists, or search bars where typos are common,"
which covers all three consumers below. FlexSearch/MiniSearch are built for corpora
orders of magnitude larger (100K+ documents) than anything in this app; Fuse.js's
simplicity fits every current and anticipated consumer here.

Sources: package-comparison research on Fuse.js/FlexSearch/MiniSearch synonym support;
Apple Spotlight lexical-index documentation and `NSPreferencePane` search-keywords
precedent; this repo's own `command-palette.tsx` and `MyAgentsList.tsx`.

## 2. Shared utility: `frontend/app/util/fuzzysearch.ts`

One small, dependency-free-of-app-state module, deliberately kept a plain function
rather than a stateful class or Solid primitive — every consumer already holds its
own corpus behind a `createMemo`/signal (command registry, settings index, agent
rows), so the utility doesn't need to manage reactivity itself, just do the matching:

```ts
import Fuse, { type IFuseOptions } from "fuse.js";

// Shared defaults tuned once for "interactive search over a small, known
// corpus" — permissive enough for typos without being noisy, matches
// anywhere in the string (not just the start). Callers override/extend
// `keys` per domain; `threshold`/`ignoreLocation`/`minMatchCharLength` are
// meant to stay consistent app-wide so search *feels* the same everywhere.
export const DEFAULT_FUZZY_OPTIONS: Partial<IFuseOptions<unknown>> = {
    threshold: 0.35,
    ignoreLocation: true,
    minMatchCharLength: 2,
};

/**
 * Fuzzy-search `items` for `query`, ranked by relevance. Returns `items`
 * unchanged (original order) when `query` is empty — "not searching," not
 * "searching, zero results." Builds a fresh Fuse index per call: every
 * corpus in this app today (commands ~25, settings ~59, an individual
 * user's agent list) is small enough that this is not a measurable cost —
 * if a future consumer's corpus grows large enough to change that
 * calculus, hold a memoized `Fuse` instance directly instead of reaching
 * for this convenience wrapper (it's a thin function, not a required
 * abstraction layer).
 */
export function fuzzySearch<T>(items: T[], query: string, options: IFuseOptions<T>): T[] {
    const q = query.trim();
    if (!q) return items;
    const fuse = new Fuse(items, { ...DEFAULT_FUZZY_OPTIONS, ...options });
    return fuse.search(q).map((r) => r.item);
}
```

Callers supply their own `keys` (which fields, what weights — the part that's
genuinely domain-specific) and use their own `createMemo(() => fuzzySearch(items(), query(), {keys: [...]}))`.
No new reactive primitive to learn — this is exactly the shape `command-palette.tsx`'s
`filtered` memo already had, just with the matching swapped out.

## 3. Consumer 1: Settings pane (the original ask)

### 3.1 Current state (verified against the code)

- `frontend/app/view/settings/` — viewType `"settings"`
  (`settings-model.ts:28`). Left rail + body, six sections driven by
  `RAIL`/`SettingsSection` (`settings-view.tsx:49-56,110-127`): Appearance, Window &
  Panes, Terminal, Sounds, Recording, Advanced — one file per section under
  `sections/`.
- Every row is `<SettingRow label description control>` (`settings-controls.tsx:17-29`)
  — inline JSX, no `id`. Rough count via `<SettingRow>` usage:
  appearance 12, window-panes 4, terminal 14, sounds 13, recording 7, advanced 9 —
  **≈59 settings total**.
- **No central registry exists.** `SettingsSection`/`SETTINGS_SECTION_LABELS`
  (`settings-model.ts:6-25`) covers only the 6 section tabs, not individual rows —
  search needs a data source that exists independent of which section is currently
  mounted, and today nothing does.
- `SettingsViewModel` (`settings-model.ts:27-53`) uses a plain `createSignal` for
  `activeSection` (no `blockAtom` — documented as intentional at `:46-50`). A `query`
  signal for search fits the same way.

### 3.2 A registry, colocated with each section's JSX (not a separate duplicate list)

The risk with "add a parallel data file listing every setting's label" is drift: two
places say what a setting is called, and nothing stops them disagreeing. Instead, each
section file exports its own typed array **above** the component, and the JSX reads
labels/descriptions from it instead of inlining the strings twice:

```ts
// frontend/app/view/settings/sections/appearance.tsx
export interface SettingsIndexEntry {
    id: string;               // "appearance.theme" — stable, used for scroll-to-result
    label: string;
    description?: string;
    section: SettingsSection;
    keywords: string[];       // curated synonyms/alt-terms — see §3.4
}

export const APPEARANCE_SETTINGS: SettingsIndexEntry[] = [
    {
        id: "appearance.theme",
        label: "Theme",
        description: "Choose light, dark, or match your system.",
        section: "appearance",
        keywords: ["dark mode", "light mode", "color scheme", "appearance mode"],
    },
    // ...
];
```

The component then does `<SettingRow label={APPEARANCE_SETTINGS[0].label} description={APPEARANCE_SETTINGS[0].description} control={...} />`
— control wiring (the bespoke toggle/slider/masked-key logic) stays exactly as it is
today; only the label/description strings move to live in one place. This eliminates
drift architecturally (there is only one string) rather than detecting it after the
fact with a lint gate.

`frontend/app/view/settings/settings-index.ts` (new, small) aggregates all six:

```ts
export const SETTINGS_INDEX: SettingsIndexEntry[] = [
    ...APPEARANCE_SETTINGS,
    ...WINDOW_SETTINGS,
    ...TERMINAL_SETTINGS,
    ...SOUNDS_SETTINGS,
    ...RECORDING_SETTINGS,
    ...ADVANCED_SETTINGS,
];
```

Plain module-level arrays exist on import, independent of which section is currently
mounted. Each `<SettingRow>`'s root element also gets `id={`setting-${entry.id}`}` (or
a `data-setting-id` attribute) so a search result can `scrollIntoView` + briefly
highlight it after switching to the right section (§3.5).

### 3.3 Matching config

```ts
const results = createMemo(() =>
    fuzzySearch(SETTINGS_INDEX, query(), {
        keys: [
            { name: "label", weight: 0.45 },
            { name: "keywords", weight: 0.35 },
            { name: "description", weight: 0.2 },
        ],
    }).slice(0, 8)
);
```

`keywords` carries the highest practical weight after `label` itself — that's
deliberately where the "synonyms match" requirement lives (§3.4), not in a separate
mechanism.

### 3.4 Curating `keywords` — the actual "synonym" content

This is where the real work is, and it's editorial, not architectural. Guidance for
authoring each entry's `keywords`:

- Include the setting's underlying config key if it differs from the label (e.g. a
  "Font Ligatures" row whose config key is `term:ligatures` should list `"ligatures"`
  explicitly).
- Include the common alternate/competitor term for the same concept (theme →
  "dark mode"/"light mode"; idle timeout → "auto-lock"/"sleep").
- Include the verb/goal phrasing a user might type instead of the noun ("make text
  bigger" for a font-size slider), where it's a short, obvious addition — not an
  attempt at full natural-language coverage (see §7's scoping of that).
- One good way to bootstrap this list: ask an LLM once, per setting, "what would a
  user type looking for this setting but not knowing its exact name?" and hand-review
  the suggestions before committing them. This is a **one-time, offline authoring
  aid**, not a runtime dependency — the shipped keyword list is static data reviewed
  and committed like any other code, with no LLM call in the search path itself.

### 3.5 Search input + result UX

New `<SettingsSearchBar>` at the top of `settings-view.tsx`'s body, wired to a
`query`/`setQuery` signal added to `SettingsViewModel`. When `results()` is non-empty
(and the query is non-empty), render a dropdown list under the input (same visual
family as the command palette's result list). Selecting a result (click, or Enter on
the highlighted one):
1. `setSection(result.section)`.
2. `queueMicrotask` to `document.getElementById(`setting-${result.id}`)?.scrollIntoView({block:"center"})`
   and add a brief CSS highlight class (removed after ~1.5s via `setTimeout`).
3. Clear the query.

Keyboard: arrow up/down to move a highlighted-result index (clamped, same pattern as
the command palette's own `clampedIdx`), Enter to select, Escape to clear and return
focus to the section body. No autofocus on pane open — matching
`SPEC_AGENT_PICKER_FILTER_SEARCH_2026_08_17.md`'s own reasoning (Q2 there): a settings
pane is opened primarily to look at/change something already visible, not to type
first.

## 4. Consumer 2: Command palette (migrated)

`frontend/app/modals/command-palette.tsx:33-43`'s `filtered` memo currently does:

```ts
return all.filter(
    (cmd) => cmd.label.toLowerCase().includes(q) || cmd.id.toLowerCase().includes(q) || cmd.category.toLowerCase().includes(q)
);
```

Replace with:

```ts
const filtered = createMemo(() => {
    const q = query().trim();
    const all = sortCommands(commandRegistry.all());
    if (!q) return all; // browsing, unfiltered: keep today's category+label order
    return fuzzySearch(all, q, {
        keys: [
            { name: "label", weight: 0.6 },
            { name: "category", weight: 0.25 },
            { name: "id", weight: 0.15 },
        ],
    }); // Fuse's own relevance order while actively searching — a deliberate
        // change from today's "still category+label sorted even mid-search"
        // behavior, and the better UX: best match first, same as every other
        // command palette (VS Code included) already does.
});
```

`clampedIdx`/keyboard nav/selection are untouched — they operate on whatever
`filtered()` returns regardless of how it was computed.

## 5. Consumer 3: Agent picker filter bar (migrated)

`frontend/app/view/agent/components/MyAgentsList.tsx:817-825`'s `filteredRows` memo
currently does:

```ts
return all.filter((r) => (r.instance_name || r.definition_name).toLowerCase().includes(q));
```

Replace with:

```ts
const filteredRows = createMemo(() => {
    const q = nameQuery();
    const all = rowsStore.list;
    if (!q) return all;
    return fuzzySearch(all, q, {
        keys: [{ name: "searchName", getFn: (r) => r.instance_name || r.definition_name }],
    });
});
```

Fuse.js's function-based key (`getFn`) computes the same `instance_name ||
definition_name` fallback the current code already does, without needing to
pre-decorate each row with a synthetic field. Unlike Settings, there's no `keywords`
concept here — agent names are user-assigned free text, not a fixed vocabulary, so
this consumer gets typo/reorder tolerance from the shared utility, not synonym
matching. That's expected and fine: the utility's value here is "the same matching
feel everywhere in the app," not synonyms specifically.

## 6. Test plan

- `fuzzysearch.test.ts` (new): a small "golden set" of `{items, query,
  expectedTopId}` cases covering literal match, typo tolerance, and — the point of
  the Settings consumer — synonym matches via `keywords` ("dark mode" →
  `appearance.theme`). Exercises the real `fuzzySearch()` call, not a mock.
- Command palette: existing tests (if any) continue to assert on `filtered()`'s
  output shape; add one case confirming mid-search results are relevance-ordered
  (the documented behavior change in §4), not category+label ordered.
- `MyAgentsList.test.tsx` (existing file, confirmed present): extend with a
  typo-tolerance case now possible post-migration; confirm the `instance_name ||
  definition_name` fallback still behaves identically for exact-match cases (no
  behavior change there, just implementation).
- Settings: a render test confirming a result's `id` resolves to a real DOM element
  after section-switch (catches the `id={`setting-${id}`}` / registry `id` field
  drifting apart, since that link isn't type-checked).

## 7. Explicitly out of scope

- **Building search UI for Swarm or Toolchain now.** Neither currently has a
  user-facing text search/filter (checked both — their existing `.filter()` calls
  are status/kind/drift filters, not query search), so there's nothing to migrate
  yet. Named here because the utility (§2) is deliberately generic rather than
  settings-shaped specifically so that adding search to either later is "import
  `fuzzySearch`, supply `keys`," not a redesign.
- **Full natural-language intent matching** ("make everything look nicer" →
  Appearance section). The curated-keyword approach (§3.4) captures short, obvious
  verb/goal phrasings as explicit keywords, which covers a meaningful chunk of this
  for free, but it is not general NLP.
- **Runtime/live LLM-backed search.** Ruled out in §1/§3.4 on latency and dependency
  grounds.
- **Searching across domains** (e.g. one box that searches settings + commands +
  agents at once). Each consumer keeps its own search bar and corpus; the shared
  piece is the matching utility, not a unified index.
- **Cross-instance or synced settings search.**

## 8. Open questions (need an answer before implementation)

**Q1: Does a Settings search result replace the section body entirely, or overlay it
as a dropdown?** §3.5 recommends a dropdown overlay (command-palette-style) so the
current section stays visible underneath. Flagging because the alternative (replace
the body with a flat list of all matches) is also reasonable and a different enough
layout to decide before writing the component.

**Q2: Where do the ~59 initial `keywords` lists come from — authored in this PR, or a
follow-up?** Recommendation: author real keyword lists in the same PR (reviewing ~59
short lists once is cheaper than an ongoing "fill in later" TODO that's easy to let
rot).
