# SPEC: Dynamic pane titles for rail/section panes (Armory, Warden, Settings)

**Date:** 2026-08-12 (extended 2026-08-13 to cover Warden and Settings — same
mechanism, originally scoped to Armory only).
**Status:** Draft — spec only, no code written yet (per explicit request).
**Verify before acting:** all file:line citations below checked against `main`
@ `e7764f780` on 2026-08-13. Re-verify if this doc is read more than a few
days later.

---

## 0. Ask

> in the armory pane, we want to change the pane title depending on the menu
> item selected. take a look, write a spec to file

Follow-up:

> get latest, i believe warden also adopted the same scaffolding as armory.
> should we do warden too?

> there is also the settings pane

---

## 1. Scope — three panes, one mechanism, one outlier

Three view types have a left-rail/bottom-tab-bar "section" selector with a
static `viewName`. Surveyed all three against source:

| Pane | Model file | Section state lives... | Has `blockAtom`? | Has `zoomAtom`? | Fix shape |
|---|---|---|---|---|---|
| **Armory** | `frontend/app/view/armory/armory-model.ts` | in the **view** (`armory-view.tsx:26`, local `createSignal`) | yes | yes (`term:zoom`) | move state into model, meta-backed (§3.1) |
| **Warden** | `frontend/app/view/warden/warden-model.ts` | in the **view** (`warden-view.tsx:26`, local `createSignal`) | yes | yes (`term:zoom`) | identical to Armory (§3.1) — near-verbatim clone, comments in source even cite Armory as the precedent |
| **Settings** | `frontend/app/view/settings/settings-model.ts` | already on the **model** (`activeSection`/`setSection`, plain `createSignal` in the constructor) | **no** | **no** | smaller fix — wire `viewName` to the signal that's already there (§3.2) |

`toolchain-model.ts` also has a static `viewName = () => "Toolchain"` but was
checked and does **not** have a rail/section pattern (single view, different
local state for env/path display) — out of scope, not a fourth instance.

Armory and Warden are close enough to be templates of each other — same file
shapes (`*.tsx` barrel / `*-model.ts` / `*-view.tsx`), same `bundle-manager-*`
CSS classes, same `RAIL` array shape, same unpersisted local signal, same
`zoomAtom` wiring. Settings shares the rail *UI* pattern but its model is
structured differently (no `blockAtom` at all — it's a leaner view model than
the other four zoom-supporting panes), so it gets its own subsection
throughout rather than being folded silently into the Armory/Warden design.

---

## 2. Current state (audited against source)

### 2.1 Armory

`armory-model.ts:22-23`:

```ts
viewIcon = () => "vault";
viewName = () => "Armory";
```

`armory-view.tsx:17-23`, the `RAIL` array (five sections: Accounts, Memories,
Skills, MCP Servers, ABF) and `armory-view.tsx:26`:

```ts
const [section, setSection] = createSignal<ArmorySection>("accounts");
```

Local, view-owned, unpersisted — resets to `"accounts"` on every remount.
`ArmorySection` type: `armory-model.ts:9` — `"accounts" | "memory" | "skills"
| "mcp" | "bundles"`.

### 2.2 Warden

`warden-model.ts:21-22`:

```ts
viewIcon = () => "shield-halved";
viewName = () => "Warden";
```

`warden-view.tsx:17-23`, the `RAIL` array (five sections: Host, LAN, Internet,
Audit, Supervisor) and `warden-view.tsx:26`:

```ts
const [section, setSection] = createSignal<WardenSection>("host");
```

Same shape as Armory, down to the `bundle-manager-rail`/`bundle-manager-tab-
bar`/`bundle-manager-pane` class names (`warden-view.tsx:57,75,81,97` reuse
Armory's own CSS classes rather than introducing `warden-rail` etc.).
`WardenSection` type: `warden-model.ts:9` — `"host" | "lan" | "internet" |
"audit" | "supervisor"`. `warden-model.ts:16-19`'s own comment states the
`zoomAtom` is copied "same term:zoom metadata + clamp range as Armory/editor/
term/agent/swarm — see armory-model.ts's zoomAtom for the precedent this
mirrors exactly," confirming Warden was built as a deliberate copy of
Armory's scaffolding, title bug included.

### 2.3 Settings — structurally different, not a straight copy

`settings-model.ts:18-19`:

```ts
viewIcon = () => "cog";
viewName = () => "Settings";
```

But unlike Armory/Warden, **the section signal already lives on the model**,
`settings-model.ts:23-31`:

```ts
activeSection: () => SettingsSection;
setSection: (s: SettingsSection) => void;

constructor(blockId: string, nodeModel: BlockNodeModel) {
    this.blockId = blockId;
    this.nodeModel = nodeModel;
    const [section, setSection] = createSignal<SettingsSection>("appearance");
    this.activeSection = section;
    this.setSection = setSection;
}
```

`settings-view.tsx:59-60` just delegates to it: `const section = () =>
props.model.activeSection();`. `SettingsSection`: `settings-model.ts:6-11` —
`"appearance" | "window" | "terminal" | "sounds" | "advanced"`.
`settings-view.tsx:48-54`'s `RAIL` array supplies the five labels
("Appearance", "Window & Panes", "Terminal", "Sounds", "Advanced").

Critically, `SettingsViewModel` has **no `blockAtom` and no `zoomAtom`** —
it's the leanest of the four zoom-capable rail panes; Settings never grew
per-pane zoom or any other `blockAtom`-derived state. That absence is the
reason its fix is smaller (§3.2) but also why it can't reuse the `zoomAtom`
meta-persistence template the way Armory/Warden can — there's no existing
`blockAtom` plumbing to piggyback on.

No test file exists for Settings (`frontend/app/view/settings/` has no
`*.test.*`), unlike Armory/Warden which both have thorough
`*-view.test.tsx` coverage of the rail and zoom behavior.

### 2.4 How the block header actually renders `viewName` (applies to all three)

`frontend/app/block/blockframe.tsx:333-347`:

```ts
const viewName = createMemo(() => {
    const bd = blockData();
    if (bd?.meta?.["frame:title"]) {
        return bd.meta["frame:title"];
    }
    let name = util.useAtomValueSafe(props.viewModel?.viewName) ?? blockViewToName(bd?.meta?.view);
    ...
    return name;
});
```

rendered at `blockframe.tsx:494-501` as
`<div class="block-frame-view-type">{viewName()}</div>` (or a
`<ViewNameEditor>` when the model implements `setViewName` — **none of
Armory, Warden, or Settings do**, so there's currently no user-facing rename
UI for any of these three panes; §5 revisits what that means for the
`frame:title` override).

Two things matter for all three panes equally:

- **`viewModel.viewName` is invoked inside a `createMemo` on every render,
  fully reactively** — any signal read inside the arrow function is tracked.
  A plain `viewName = () => someAccessor()` is sufficient; extra
  memoization (`useBlockAtom` + `createMemo`) is only needed when the
  accessor itself has no tracking owner of its own (§3.1 vs §3.2 below).
- **`frame:title` block-meta wins over `viewModel.viewName()` unconditionally**
  — this already works for Agent's dynamic agent-name title with zero
  special-case code, and will for these three the same way (§5).

### 2.5 Existing precedent for reactive `viewName` (applies to all three)

- **`editor-model.ts:352-363`** — needs `useBlockAtom` wrapping because its
  source values (`filePathAtom`, `dirtyAtom`) are themselves derived memos
  with no owner otherwise:
  ```ts
  this.viewName = useBlockAtom(blockId, "editor-view-name", () =>
      createMemo<string>(() => {
          const fp = this.filePathAtom();
          if (!fp) return "Editor";
          return this.dirtyAtom() ? `${fp} *` : fp;
      }),
  );
  ```
- **`agent-model.ts:112-117`** — no wrapper needed, reads `this.blockAtom()`
  (already a tracked signal) directly in a plain arrow function:
  ```ts
  this.viewName = () => {
      const meta = this.blockAtom()?.meta;
      const name = meta?.["agentName"];
      return typeof name === "string" && name.length > 0 ? name : "Agent";
  };
  ```
- **Armory/Warden's own `zoomAtom`** (`armory-model.ts:31-37`,
  `warden-model.ts:30-36`, byte-for-byte identical) — the direct template for
  a *second* meta-backed, `useBlockAtom`-wrapped accessor in the same
  constructor, just keyed on `armory:section`/`warden:section` instead of
  `term:zoom`.

**Which pattern each pane needs:** Armory/Warden's new `sectionAtom` needs the
`useBlockAtom` wrapper (same reason as Editor — it's a derived memo over
`blockAtom().meta`, with no other owner). Settings' `activeSection` is already
a **plain `createSignal`**, not a derived memo — calling it directly from a
bare `viewName = () => ...` arrow function is exactly the Agent pattern; no
wrapper needed at all (§3.2).

### 2.6 No "Parent: Child" title convention exists

No `viewName` implementation in the codebase formats a two-part title (e.g.
`"Armory: Skills"`). Every dynamic title is a full replacement (Agent shows
just the agent's name; Editor shows just the file path). `.block-frame-view-
type` has width/ellipsis constraints (`blockframe.tsx:452-454` comment), so
titles should stay short regardless of which format is chosen for these three
(§6.2).

---

## 3. Design

### 3.1 Armory and Warden — move `section` state into the model, meta-backed

Both need the same fix, applied per-file. Using Armory as the worked example
(Warden is a 1:1 substitution — `warden`/`Warden`/`WardenSection` for
`armory`/`Armory`/`ArmorySection`, `armory:section` → `warden:section`):

**`armory-model.ts`** — add a `sectionAtom`, mirroring `zoomAtom` exactly:

```ts
sectionAtom: Accessor<ArmorySection>;
...
this.sectionAtom = useBlockAtom(blockId, "armory-section", () =>
    createMemo<ArmorySection>(() => {
        const s = this.blockAtom()?.meta?.["armory:section"];
        return isArmorySection(s) ? s : "accounts";
    }),
);
```

(`isArmorySection`/`isWardenSection` — a small type guard against the five
valid ids per pane, so a stale/garbage meta value can't crash the rail into
an unknown section; see §4.)

Then `viewName`, following the Editor pattern (§2.5):

```ts
this.viewName = useBlockAtom(blockId, "armory-view-name", () =>
    createMemo<string>(() => ARMORY_SECTION_LABELS[this.sectionAtom()]),
);
```

**`armory-view.tsx`** — replace the local `createSignal` with reads/writes
through the model, using the exact same `RpcApi.SetMetaCommand` shape already
used for the wheel-zoom handler (`armory-view.tsx:39-50`,
`warden-view.tsx:35-46`):

```ts
const section = model.sectionAtom;
const setSection = (id: ArmorySection) =>
    void RpcApi.SetMetaCommand(TabRpcClient, {
        oref: `block:${model.blockId}`,
        meta: { "armory:section": id },
    });
```

Both `<For>` blocks (rail + tab bar) keep calling `setSection(item.id)`
unchanged in both files — only the implementation moves.

**Why meta-persist over a view-local-lifted-to-model-but-unpersisted signal:**
the meta write is one extra cheap fire-and-forget RPC per section switch
(identical cost to the existing zoom writes), and it buys a real UX
improvement for free — **the selected tab now survives block remount** (tab
close/reopen, app restart, layout drag), where today it silently resets every
time. Armory's own zoom already does this; nothing about either pane's
section is more sensitive than its zoom level. If a reviewer prefers a
smaller diff without persistence, the alternative is a plain `createSignal`
promoted into the model's constructor (no `useBlockAtom`/meta) — `viewName`'s
wiring is identical either way; only where `section`'s source-of-truth comes
from changes. Flagged as a recommended-not-mandatory call in §6.1.

**Label tables — avoiding the circular-import trap:** `armory.tsx`/
`warden.tsx` exist *only* to break a circular import between the `*-model.ts`
and `*-view.tsx` pair (model needs the view as `viewComponent`; view needs the
model's types). If `*-model.ts` also needs the id→label mapping that today
lives inline in each `*-view.tsx`'s `RAIL` array, importing it from the view
file would reintroduce exactly the cycle the barrel file was built to avoid.
Fix: hoist the id→label pairs (label text only — icon/tooltip stay view-only
presentation detail) into each `*-model.ts`, next to the `*Section` type it
already owns:

```ts
// armory-model.ts
export const ARMORY_SECTION_LABELS: Record<ArmorySection, string> = {
    accounts: "Accounts",
    memory:   "Memories",
    skills:   "Skills",
    mcp:      "MCP Servers",
    bundles:  "ABF",
};
```

```ts
// warden-model.ts
export const WARDEN_SECTION_LABELS: Record<WardenSection, string> = {
    host:       "Host",
    lan:        "LAN",
    internet:   "Internet",
    audit:      "Audit",
    supervisor: "Supervisor",
};
```

Each `RAIL` array then references its model's `*_SECTION_LABELS[id]` for the
`label` field instead of duplicating the string literal, so the two can never
drift out of sync (precedent for this class of drift: the "Memory" →
"Memories" rename in
`SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13.md` only
touched one file at the time; a future rename now only needs to touch the
`*_SECTION_LABELS` constant).

### 3.2 Settings — smaller fix, no state to move

Settings' `activeSection` already lives in the right place (the model), so
this is **not** a copy of §3.1 — there's no state-ownership gap to close, and
(per §2.3) no `blockAtom` to hang meta-persistence off without first building
that infrastructure from scratch. The fix is one line:

```ts
// settings-model.ts
viewName = () => SETTINGS_SECTION_LABELS[this.activeSection()];
```

placed after `this.activeSection = section;` in the constructor (needs
`this`, so it can't stay a class-field arrow initializer above the
constructor the way the current static `viewName = () => "Settings"` is —
same relocation `agent-model.ts:112` already does for the same reason). No
`useBlockAtom`/`createMemo` wrapper needed: `activeSection` is a plain
`createSignal` getter, and `viewName` here is a plain arrow function invoked
inside `blockframe.tsx`'s own `createMemo` — exactly the Agent pattern
(§2.5), which needs no owner of its own because it doesn't declare a new
derived computation, it just re-reads an existing signal on each outer
re-evaluation.

`SETTINGS_SECTION_LABELS` — same circular-import reasoning as §3.1, hoisted
into `settings-model.ts`:

```ts
export const SETTINGS_SECTION_LABELS: Record<SettingsSection, string> = {
    appearance: "Appearance",
    window:     "Window & Panes",
    terminal:   "Terminal",
    sounds:     "Sounds",
    advanced:   "Advanced",
};
```

`settings-view.tsx`'s `RAIL` array (`settings-view.tsx:48-54`) references it
for `label` instead of the inline literals.

**No persistence, by design, not by oversight:** Settings' section resets to
`"appearance"` on remount today and will continue to after this fix — adding
meta-persistence here would mean introducing `blockAtom` to a view model that
has deliberately never needed one, a materially bigger and separable change
from "make the title reactive." If persistence-on-remount is wanted for
Settings too, it's a legitimate follow-up, but bundling it into this fix would
make Settings' diff disproportionately larger than Armory/Warden's for a
UX property that wasn't asked for. Flagged as an open call in §6.1.

### 3.3 Icons: out of scope for all three

`viewIcon` stays static (`"vault"`, `"shield-halved"`, `"cog"`). The ask was
specifically about the *title*; each `RAIL` already carries a distinct icon
per section shown in the rail/tab-bar itself, so a per-section pane-header
icon would be a separate, follow-up decision per pane (§7).

---

## 4. Edge cases (apply identically to Armory, Warden, and — persistence rows
excepted per §3.2 — Settings)

| Case | Behavior after this change |
|---|---|
| Fresh block, no section meta yet (Armory/Warden) / fresh model (Settings) | Defaults to the first rail item — "Accounts" (Armory), "Host" (Warden), "Appearance" (Settings) |
| Click a different rail item or bottom tab | State updates → title recomputes → header re-renders immediately (meta round-trip for Armory/Warden; synchronous signal read for Settings) |
| User manually renames the pane (`frame:title` set via some future `setViewName`) | `frame:title` wins unconditionally (`blockframe.tsx:335-336`) — dynamic title suppressed until cleared. None of these three panes implement `setViewName` today, so this path isn't reachable from the UI yet for any of them; documented for when/if it is (§5). |
| Two panes of the same type open in different blocks/tabs | Armory/Warden: independent `blockId`-scoped meta (`useBlockAtom` keys off `blockId`, same as `zoomAtom` today). Settings: independent model instances, same as today — no cross-instance sharing regardless of this change. |
| Block remounts (tab reopen, app restart, drag-to-new-layout) | Armory/Warden: previously selected section — and its title — now persist (new behavior, §3.1). Settings: resets to "Appearance" (unchanged, §3.2). |
| Malformed/stale section meta value (Armory/Warden only — e.g. a hand-edited `settings.json`, or a future removed section id) | `isArmorySection`/`isWardenSection` guard rejects it, falls back to the default section — same defensive shape as `zoomAtom`'s `NaN`/type check. Not applicable to Settings (no meta involved). |
| Existing rail-ordering/label tests (`armory-view.test.tsx:85-90`, `warden-view.test.tsx:56-61`) | Unaffected — rendered label text is unchanged, only its *source* (inline string → `*_SECTION_LABELS` constant) moves |

---

## 5. Interaction with `frame:title` (manual rename) — confirmed non-issue, currently inert

`frame:title` block-meta already takes unconditional priority over any
`viewModel.viewName()` (`blockframe.tsx:335-336`), for every view type,
including these three once implemented — this already works for Agent's
dynamic agent-name title with zero special-case code. Note from §2.4: none of
Armory, Warden, or Settings currently implement `setViewName`, so there is no
existing UI path (double-click title, or otherwise) that actually sets
`frame:title` on one of these three panes today — this section documents
correct *future-proofing* behavior (if `setViewName` is ever added to one of
them), not a currently-exercised interaction.

---

## 6. Resolved / open design decisions

1. **Meta-persist Armory/Warden's `section` (like zoom) vs. keep it
   view-local-but-model-owned? — recommended: meta-persist (§3.1).** Reuses
   the exact established pattern already in the same file (`zoomAtom`), one
   extra cheap RPC per switch, tab-survives-remount as a free correctness
   improvement. Product-taste call, not a technical constraint.
2. **Should Settings get the same meta-persistence, for consistency across
   all three panes? — open, leaning no.** §3.2's reasoning: Settings has no
   existing `blockAtom`, so matching Armory/Warden here means introducing new
   infrastructure to a deliberately lean model, for a property (persistence)
   that's separable from this fix's actual ask (reactive title) and wasn't
   requested. Worth asking the reviewer explicitly rather than deciding
   unilaterally, since "all three panes behave consistently" is a reasonable
   counter-argument.
3. **Title format — bare section label ("Accounts") vs. prefixed
   ("Armory — Accounts")? — open, leaning bare label**, for all three, for
   consistency with each other and with Agent/Editor's existing full-
   replacement convention (§2.6). One-line change in each `viewName` body
   either way; doesn't affect anything else in the design.

---

## 7. Non-goals

- No change to any `viewIcon` — the ask was scoped to titles; per-section
  pane-header icons are a separate, later decision per pane (§3.3).
- No change to any `RAIL` array's icon/tooltip fields, click handling,
  `is-active` styling, or tab-bar structure — only where each `label` string
  and section state are sourced from.
- No change to which managers/sections stay mounted (all three panes'
  "everything stays mounted, toggle via `is-hidden`"/`<Switch>` behavior is
  untouched).
- No backend/Rust changes — pure frontend, meta-key-scoped for Armory/Warden
  (`armory:section`, `warden:section`, alongside the existing `term:zoom`
  reuse), no wire-format change at all for Settings.
- No new `blockAtom`/zoom support added to `SettingsViewModel` as a side
  effect of this fix (§3.2, §6.2) — that would be a separate, larger spec if
  wanted.

---

## 8. Files touched

**Armory:**
- `frontend/app/view/armory/armory-model.ts` — add `ARMORY_SECTION_LABELS`,
  `isArmorySection`, `sectionAtom` (meta-backed, mirrors `zoomAtom`), change
  `viewName` to a `useBlockAtom`-wrapped memo (mirrors `editor-model.ts`).
- `frontend/app/view/armory/armory-view.tsx` — replace the local
  `createSignal<ArmorySection>` (line 26) with `model.sectionAtom`; replace
  `setSection` with an `RpcApi.SetMetaCommand` write to `"armory:section"`;
  `RAIL`'s `label` fields reference `ARMORY_SECTION_LABELS`.
- `frontend/app/view/armory/armory-view.test.tsx` — existing rail tests
  unaffected; add coverage for `viewName()` reactivity.
- New: `frontend/app/view/armory/armory-model.test.ts` — unit tests for
  `sectionAtom` defaulting/clamping and `viewName` reactivity.

**Warden (same shape, own files):**
- `frontend/app/view/warden/warden-model.ts` — add `WARDEN_SECTION_LABELS`,
  `isWardenSection`, `sectionAtom` (backed by `warden:section`), reactive
  `viewName`.
- `frontend/app/view/warden/warden-view.tsx` — replace local `createSignal`
  (line 26) with `model.sectionAtom`; meta-write `setSection`; `RAIL` labels
  reference `WARDEN_SECTION_LABELS`.
- `frontend/app/view/warden/warden-view.test.tsx` — existing tests
  unaffected; add `viewName()` reactivity coverage.
- New: `frontend/app/view/warden/warden-model.test.ts`.

**Settings (smaller diff, no new file needed for state):**
- `frontend/app/view/settings/settings-model.ts` — add
  `SETTINGS_SECTION_LABELS`; move `viewName` into the constructor as
  `this.viewName = () => SETTINGS_SECTION_LABELS[this.activeSection()]`,
  replacing the static class-field version.
- `frontend/app/view/settings/settings-view.tsx` — `RAIL`'s `label` fields
  reference `SETTINGS_SECTION_LABELS` instead of inline literals.
- New: `frontend/app/view/settings/settings-view.test.tsx` (doesn't exist
  today, §2.3) — at minimum, cover `viewName()` reactivity across all five
  sections; optionally backfill the rail-ordering/click coverage Armory and
  Warden already have, since this fix touches the same file and there's
  currently zero test coverage of `SettingsView` to catch a regression.

---

## 9. Test plan

**Unit — Armory/Warden (same shape, own files):**

- `sectionAtom` defaults to the first section when meta is unset.
- `sectionAtom` reflects a valid meta value.
- `sectionAtom` falls back to the default for an invalid/unknown meta value.
- `viewName()` returns the matching label for each of the five sections.
- Clicking a rail button (or bottom tab bar button) calls
  `RpcApi.SetMetaCommand` with `{ "<armory|warden>:section": <id> }` — same
  assertion style as the existing zoom wheel tests.
- Existing rail-ordering/label tests still pass unmodified.

**Unit — Settings:**

- `viewName()` returns the matching `SETTINGS_SECTION_LABELS` entry for each
  of the five sections, driven by calling `model.setSection(...)` directly
  (no meta/RPC involved).
- (New baseline coverage, since none exists) rail renders all five labels in
  order; clicking a rail/tab-bar item updates `activeSection()` and swaps the
  visible `<Match>` section.

**Manual / integration (`task dev`), per pane:**

- Open the pane; confirm the header shows the first section's label
  initially.
- Click each rail item and each bottom tab in turn; confirm the header title
  updates to match immediately, for both click surfaces.
- Armory/Warden only: close and reopen the tab (or restart the app); confirm
  the previously selected section — and its title — persist.
- Settings only: close and reopen the tab; confirm it resets to "Appearance"
  (expected, §3.2 — not a regression).
- Open two panes of the same type side by side; confirm switching one's
  section does not affect the other's title.
