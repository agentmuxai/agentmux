# SPEC: Agent Pane Header Color Theme (Right-Click Picker)

**Date:** 2026-06-23
**Status:** Draft
**Scope:** Frontend-only — no Rust changes, no schema changes
**Files touched:**
- `frontend/app/view/agent/agentModel.ts` (or the agent ViewModel file)
- `frontend/app/block/blockframe.tsx`
- `frontend/app/block/block.scss`

---

## 1. Problem

Agent panes all look identical in a layout with multiple agents. There is no visual
affordance to distinguish "agent for feature A" from "agent for code review" at a
glance. Users working with 3–6 concurrent agents lose orientation.

The pane header already has a dynamic background path (`agentColor()` from the agent's
env var), and the focus outline already reads `frame:activebordercolor` from block meta.
Both plumbing hooks exist — they just aren't exposed to users.

---

## 2. UX Flow

1. User right-clicks the agent pane header.
2. The existing context menu appears (already has: Copy BlockId, Edit Title, Magnify,
   Close, Split…). A new entry **"Pane Color ▶"** is appended to the settings section.
3. The submenu lists 9 choices: **None** + 8 named hues. The currently active choice
   is checked (radio semantics).
4. Clicking a hue: writes `frame:hue` (integer 0–360) to the block's meta via
   `SetMetaCommand`. The header background and active-border update reactively within one
   frame. No reload.
5. Clicking **None**: writes `frame:hue = null`, removing the tint. Header reverts to
   the env-var color (or plain `--main-bg-color`).

The submenu is rendered via the existing `ContextMenuModel.showContextMenu()` → native
OS context menu path. No new UI component is needed.

---

## 3. Color Palette

Eight named hues covering the perceptual rainbow, spaced to remain visually distinct
at the dark saturation used for the header background.

| Name     | H   | Header bg (`hsl(H, 28%, 16%)`) | Active border (`hsl(H, 65%, 52%)`) |
|----------|-----|-------------------------------|-------------------------------------|
| Cobalt   | 218 | `#1d2433`                     | `#3d87e0`                           |
| Emerald  | 150 | `#183026`                     | `#28c471`                           |
| Amber    | 38  | `#2f2519`                     | `#d98c1f`                           |
| Rose     | 352 | `#2f1a1e`                     | `#e03558`                           |
| Violet   | 270 | `#201a30`                     | `#9451d6`                           |
| Cyan     | 188 | `#172c30`                     | `#1dc9e0`                           |
| Coral    | 14  | `#2f2019`                     | `#e05c28`                           |
| Mint     | 163 | `#182e28`                     | `#28d9a0`                           |

"Cobalt" (H=218) closely matches the default midnight accent (`--accent-color`) and is
the natural default choice for users who want a subtle tint without changing their
mental model of the existing theme.

---

## 4. Two-Tone Derivation Formula

Both tones share the same hue **H** read from `frame:hue`. The offset is fixed:

```
header_bg     = hsl(H,  28%,  16%)   // dark, muted — doesn't compete with content
active_border = hsl(H,  65%,  52%)   // vivid, luminous — clearly signals focus
```

**Rationale for the offsets:**
- **Lightness +36 pts** (16% → 52%): the header sits in the background; the border must
  be readable against both dark content and the dark panel chrome. L≈52% is the ISO
  "mid-tone" for perceived equal lightness on dark backgrounds.
- **Saturation +37 pts** (28% → 65%): the header is a supporting surface; strong
  saturation there competes with text. The border is decorative chrome and benefits from
  full vibrancy to signal "this pane is focused".
- **No hue shift**: same H keeps the two surfaces visually unified as "the same color
  family" rather than two unrelated colors. Users read it as one palette, not two choices.

The formula is applied **entirely in the frontend** via a `createMemo` in
`blockframe.tsx`. Only the hue integer (0–360) is persisted.

---

## 5. Data Model

Single new block meta key:

| Key          | Type          | Meaning                                                   |
|--------------|---------------|-----------------------------------------------------------|
| `frame:hue`  | `number\|null` | HSL hue 0–360 for the pane color theme. `null` = none.   |

No other keys. Both derived colors are computed at render time from this one value.

**Persistence:** `frame:hue` is written via `RpcApi.SetMetaCommand(TabRpcClient, { oref, meta: { "frame:hue": H } })`.
It lives in the block object in the wave store and survives workspace reload.

**Priority:** `frame:hue` takes precedence over the existing `AGENTMUX_AGENT_COLOR`
env-var path for the header. If both are set, `frame:hue` wins (user intent > process
default). The env-var path is checked first; if `frame:hue` is also set, it overrides.

---

## 6. Files and Changes

### 6a. `frontend/app/block/blockframe.tsx`

**Header style memo** (around line 427 — `headerStyle`):

Currently:
```ts
const headerStyle = createMemo<JSX.CSSProperties>(() => {
    const style: JSX.CSSProperties = {};
    const ac = agentColor();
    const atc = agentTextColor();
    if (ac) style["background-color"] = ac;
    if (atc) style.color = atc;
    return style;
});
```

Extend to read `frame:hue` from block meta and override the background when present:
```ts
const headerStyle = createMemo<JSX.CSSProperties>(() => {
    const style: JSX.CSSProperties = {};
    const hue = blockData()?.meta?.["frame:hue"];
    if (typeof hue === "number") {
        style["background-color"] = `hsl(${hue}, 28%, 16%)`;
        // text color stays inherited (--main-text-color is light enough on L=16%)
    } else {
        const ac = agentColor();
        const atc = agentTextColor();
        if (ac) style["background-color"] = ac;
        if (atc) style.color = atc;
    }
    return style;
});
```

**Border style memo** (around line 651 — `style` / focus border logic):

The existing logic reads `frame:activebordercolor` from block meta. Extend the focused
branch to also check `frame:hue` as a fallback:
```ts
if (isFocused()) {
    const hue = bd?.meta?.["frame:hue"];
    if (typeof hue === "number") {
        style["border-color"] = `hsl(${hue}, 65%, 52%)`;
    }
    // existing frame:activebordercolor still takes precedence (checked after)
    if (bd?.meta?.["frame:activebordercolor"]) {
        style["border-color"] = bd.meta["frame:activebordercolor"];
    }
    ...
}
```

Priority chain for focused border: `frame:activebordercolor` > `frame:hue` > tab meta
`bg:activebordercolor` > CSS variable. This is consistent with the existing override
hierarchy while adding hue as a new mid-tier option.

**Context menu entry in `handleHeaderContextMenu()`** (line 41–109):

After the view-model's `getSettingsMenuItems()` block, append:
```ts
menu.push({ type: "separator" });
menu.push(buildPaneColorSubmenu(blockData(), model.blockId));
```

where `buildPaneColorSubmenu` is a new function (see §6b).

### 6b. New helper: `frontend/app/block/pane-color-menu.ts`

```ts
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { ContextMenuItem } from "@/types/custom";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { WOS } from "@/app/store/global";

interface HueOption {
    label: string;
    hue: number;
}

export const PANE_HUE_OPTIONS: ReadonlyArray<HueOption> = [
    { label: "Cobalt",  hue: 218 },
    { label: "Emerald", hue: 150 },
    { label: "Amber",   hue:  38 },
    { label: "Rose",    hue: 352 },
    { label: "Violet",  hue: 270 },
    { label: "Cyan",    hue: 188 },
    { label: "Coral",   hue:  14 },
    { label: "Mint",    hue: 163 },
];

function setHue(blockId: string, hue: number | null): void {
    void RpcApi.SetMetaCommand(TabRpcClient, {
        oref: WOS.makeORef("block", blockId),
        meta: { "frame:hue": hue },
    });
}

export function buildPaneColorSubmenu(
    blockData: Block | null,
    blockId: string,
): ContextMenuItem {
    const currentHue = (blockData?.meta?.["frame:hue"] as number | undefined) ?? null;

    const items: ContextMenuItem[] = [
        {
            label: "None",
            type: "radio",
            checked: currentHue === null,
            click: () => setHue(blockId, null),
        },
        { type: "separator" },
        ...PANE_HUE_OPTIONS.map(({ label, hue }) => ({
            label,
            type: "radio" as const,
            checked: currentHue === hue,
            click: () => setHue(blockId, hue),
        })),
    ];

    return {
        label: "Pane Color",
        type: "submenu",
        submenu: items,
    };
}
```

### 6c. `frontend/app/block/block.scss`

The block border already transitions on focus. Ensure the CSS `border-color` transition
covers the new dynamic value (no change likely needed — the existing rule is on the
`.block-frame-default` element and applies to any `border-color` source).

If a subtle colored tint on the **unfocused** header is desired (weaker version when
not focused), that is opt-in and out of scope for v1. The tint applies whether focused
or not; only the border changes with focus.

---

## 7. Interaction with Existing Color Paths

| Source                    | Header bg          | Active border      | Unfocused border   |
|---------------------------|--------------------|--------------------|--------------------|
| `frame:hue` (new)         | `hsl(H, 28%, 16%)` | `hsl(H, 65%, 52%)` | unchanged (theme)  |
| `frame:activebordercolor` | —                  | overrides hue      | —                  |
| `AGENTMUX_AGENT_COLOR`    | used if no hue     | —                  | —                  |
| Tab `bg:activebordercolor`| —                  | lowest priority    | —                  |
| CSS variables             | `--main-bg-color`  | `--accent-color`   | `--border-color`   |

The unfocused pane does **not** show the vivid border tone — that would cause color
noise when many colored panes are open. The header tint is always visible; the vivid
border only appears when focused. This is the same pattern as VS Code's tab colorization.

---

## 8. Edge Cases

- **Floating pane**: `frame:hue` is in the block meta which the floating renderer
  reads from the same wave store. No special handling needed.
- **Magnified pane**: header is still visible; tint applies normally.
- **Minimized pane** (from pane-minimize PR #1726): the minimized header stub still
  reads `blockData()`, so the tint applies to the collapsed header bar as well —
  useful for identifying collapsed agents.
- **Swarm row**: the Swarm view derives its `agentStatus` from the model, not block
  meta. The Swarm active-row highlight (from `feature/swarm-active-sync`) uses
  `--accent-color`. A follow-up can derive the Swarm row highlight from `frame:hue`
  for full visual consistency, but that is out of scope for v1.
- **Multiple panes, same hue**: allowed. Two agents can share a color; that is the
  user's choice.
- **Workspace reload**: `frame:hue` survives in the wave store. Color restores
  automatically on reload.

---

## 9. Out of Scope

- **Custom hue picker** (drag slider to arbitrary H): the 8-hue palette is the v1
  constraint. A full color picker could be added later as an additional "Custom…" menu
  item that opens a modal.
- **Tone customization** (letting users tune S or L offsets): fixed offsets are the
  correct UX for v1 — the formula is the feature, not the knob.
- **Swarm row color sync**: follow-up ticket after v1 ships.
- **Non-agent pane types**: the submenu is added to the agent pane context menu only
  (via the agent ViewModel or directly in `handleHeaderContextMenu` scoped to
  `blockData()?.meta?.["view"] === "agent"`). Other pane types (browser, terminal,
  swarm) are unaffected in v1.
- **Tab-level hue** (color all panes in a tab): separate concept, separate spec.

---

## 10. Implementation Order

1. **`pane-color-menu.ts`** — pure data + helper; no dependencies on step 2.
2. **`blockframe.tsx` header style** — extend `headerStyle` memo.
3. **`blockframe.tsx` border style** — extend focus border memo.
4. **`blockframe.tsx` context menu** — wire `buildPaneColorSubmenu` into
   `handleHeaderContextMenu()`.

All changes are additive. No existing menu items are removed.

---

## 11. Verification Checklist

- [ ] Right-click agent pane header → "Pane Color ▶" submenu appears
- [ ] Submenu lists: None (checked if no hue), separator, Cobalt, Emerald, Amber,
      Rose, Violet, Cyan, Coral, Mint
- [ ] Selecting a hue: header background immediately tints to `hsl(H, 28%, 16%)`
- [ ] Selecting a hue: focused active border shows `hsl(H, 65%, 52%)`
- [ ] Unfocused border stays at theme default (no vivid color leak)
- [ ] Selecting None: header reverts to `agentColor()` or `--main-bg-color`
- [ ] After workspace reload: selected hue persists on the pane
- [ ] Two panes with different hues: each shows its own color independently
- [ ] Minimized pane (if pane-minimize is merged): collapsed header still shows tint
- [ ] `frame:activebordercolor` in meta still overrides the hue-derived border
- [ ] `AGENTMUX_AGENT_COLOR` env var applies when `frame:hue` is null
- [ ] Context menu on non-agent pane (terminal, browser): "Pane Color" does NOT appear
