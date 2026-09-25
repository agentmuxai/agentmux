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
    /**
     * Whether a loading cover is on screen over this pane RIGHT NOW, for any
     * reason (SPEC_PANE_LOADING_CONSOLIDATION_2026_09_20.md). Read it to avoid
     * rendering a SECOND loading affordance underneath one — the acceptance
     * criterion is a count: never more than one at a time.
     *
     * Deliberately "is a cover up", not "is the controller assembling". An
     * earlier version passed `PaneReadiness` and asked `isLoading()`, which
     * only knows the controller's own phase and is permanently false once the
     * pane goes live — blind to the separate re-cover cycle a cyclic signal
     * drives (see block.tsx). A mid-life suspension during one of those
     * re-covers then rendered a spinner underneath the cover: the exact
     * two-brains case this consolidation removes. (reagent P1 on #3466.)
     *
     * Optional because `BlockPreview` (a static drag thumbnail) is never
     * covered.
     */
    covered?: () => boolean;
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

    // NOTE: there is deliberately no `isLoading` here yet.
    // SPEC_PANE_LOADING_CONSOLIDATION_2026_09_20.md §5.4 asks BlockFrame to
    // subscribe to pane readiness and suppress transient header affordances.
    // Wiring it through these props does not work, and does so SILENTLY: every
    // registered view type gets the shared chrome (`pane-leaf-chrome.tsx`'s
    // `hoistsOwnChrome`), which suppresses BlockFrame's own inline header; the header those panes
    // actually show is built by `PaneHeaderTabStrip`, which lives OUTSIDE
    // `<Block>` and constructs its own explicit prop object. A prop threaded
    // from Block can never reach it. See spec §6.1 — phase 4 needs the same
    // pane-scoped readiness handle as phase 5. (reagent P1 on #3464.)

    /** Universal Pane Tabs (SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md
     *  §4.1) — when provided, `BlockFrame_Header` renders this INSTEAD of its
     *  own `.block-frame-default-header-iconview` (icon + title + blockid),
     *  while keeping every other row element (ConnectionButton, header text
     *  elems, EndIcons) exactly as-is. PaneHeaderTabStrip always sets it,
     *  lone tab included — a Pane header always shows its tabs
     *  (SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md §8).
     *  The per-pane name, icon and rename affordance that ReAgent P1 on
     *  PR #3309 guarded here now live on the pill itself (PaneChrome's
     *  pane-tab model, #3373), so a lone pill loses nothing the iconview
     *  had. Left unset only by the non-PaneChrome BlockFrame header path,
     *  which keeps the plain iconview. */
    leadingTabStrip?: JSX.Element;

    /** Explicit header-row background, overriding the active block's own
     *  pane color (`computeBlockColorBg`) and the non-agent default.
     *
     *  Only ever set by PaneChrome, via PaneHeaderTabStrip, and only when
     *  this pane's tabs don't agree on a single color — the header row is
     *  one element shared by the pills and the leftover "tail" (the "+"
     *  and the bar), so without this the tail repaints itself with
     *  whichever tab is active. See
     *  SPEC_PANE_HEADER_TAIL_COLOR_2026_09_21.md.
     *
     *  Deliberately a resolved color string, not the tab set itself:
     *  BlockFrame_Header renders ONE block and has no business knowing
     *  what else is in the pane. */
    headerBgOverride?: string;
}
