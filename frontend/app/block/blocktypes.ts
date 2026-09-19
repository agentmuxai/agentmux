// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { NodeModel } from "@/layout/index";
import type { Accessor, JSX } from "solid-js";

// BlockNodeModel is defined globally in types/custom.d.ts.
// We redeclare a compatible local version here for use in sub-block components.
export interface BlockNodeModel {
    blockId: string;
    isFocused: Accessor<boolean>;
    disablePointerEvents: Accessor<boolean>;
    innerRect?: Accessor<{ width: string; height: string }>;
    onClose?: () => void;
    focusNode: () => void;
    /** True when this Block is rendered under `pane-leaf-chrome.tsx`'s
     *  hoisted-chrome branch, i.e. something ABOVE it already renders a
     *  replacement pane header. Absent everywhere else — notably on the
     *  raw leaf nodeModel a drag-preview thumbnail gets
     *  (`tabcontent.tsx`'s `renderPreview`), which has no chrome around it
     *  and therefore still needs `BlockFrame`'s own inline header. See
     *  that wrapper's construction in `pane-leaf-chrome.tsx`.
     *
     *  An ACCESSOR, not a plain boolean, deliberately — it's the SAME
     *  `hoisted` memo reference `pane-leaf-chrome.tsx` computes for the
     *  whole leaf, forwarded as-is rather than snapshotted. A plain
     *  boolean baked in at wrapper-construction time goes stale the
     *  moment a LATER wrapper (a different `paneChromeHoisted` snapshot)
     *  gets built for the same blockId before block.tsx's ViewModel
     *  registry has a chance to construct a fresh ViewModel for it —
     *  `getBlockComponentModel` ADOPTS the existing one instead whenever
     *  `viewType` still matches, so a `noHeader` reading a frozen
     *  snapshot off `this.nodeModel` can end up permanently wrong (the
     *  double-header bug this comment's PR fixed). Forwarding the LIVE
     *  memo instead means every wrapper for this leaf — however many get
     *  built, whichever one a ViewModel happens to be holding — always
     *  reports the SAME, currently-correct value, so ViewModel adoption
     *  timing can no longer matter. */
    paneChromeHoisted?: () => boolean;
}

export type FullBlockProps = {
    preview: boolean;
    nodeModel: NodeModel;
    viewModel: ViewModel;
};

export interface BlockProps {
    preview: boolean;
    nodeModel: NodeModel;
}

export type FullSubBlockProps = {
    nodeModel: BlockNodeModel;
    viewModel: ViewModel;
};

export interface SubBlockProps {
    nodeModel: BlockNodeModel;
}

export interface BlockComponentModel2 {
    onClick?: () => void;
    onFocusCapture?: (e: FocusEvent) => void;  // used as onFocusIn in SolidJS
    blockRef?: { current: HTMLDivElement | null };
}

export interface BlockFrameProps {
    blockModel?: BlockComponentModel2;
    nodeModel?: NodeModel;
    viewModel?: ViewModel;
    preview: boolean;

    children?: JSX.Element;
    connBtnRef?: { current: HTMLDivElement | null };

    /** Universal Pane Tabs (SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md
     *  §4.1) — when provided, `BlockFrame_Header` renders this INSTEAD of its
     *  own `.block-frame-default-header-iconview` (icon + title + blockid),
     *  while keeping every other row element (ConnectionButton, header text
     *  elems, EndIcons) exactly as-is. Only set this when there are 2+ real
     *  Pane Tabs to show as pills — with 0 or 1, the real iconview (which
     *  already correctly reads the active ViewModel's own name/icon and
     *  supports click-to-rename via `ViewNameEditor`) is strictly more
     *  correct than any synthetic substitute (ReAgent P1 on PR #3309: an
     *  earlier version of this unconditionally overrode the iconview with a
     *  hardcoded literal for the single most common pane state — a lone
     *  conversation/shell — silently losing the real per-pane name, icon,
     *  and rename affordance). See `trailingAddButton` for how the "+"
     *  still reaches the row in that case. */
    leadingTabStrip?: JSX.Element;

    /** Rendered right after the iconview/leadingTabStrip, ONLY when
     *  `leadingTabStrip` is unset (i.e. the real iconview is showing) —
     *  when `leadingTabStrip` IS set, its own tab strip already carries its
     *  own "+", so this is skipped to avoid a duplicate. This is how a
     *  lone-conversation/lone-shell Pane still gets an "add tab" affordance
     *  without losing its real identity — see `leadingTabStrip`'s own doc
     *  comment for the bug this split fixes. */
    trailingAddButton?: JSX.Element;
}
