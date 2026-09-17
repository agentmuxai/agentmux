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
     *  that wrapper's construction in `pane-leaf-chrome.tsx`. */
    paneChromeHoisted?: boolean;
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
     *  elems, EndIcons) exactly as-is. This is how `PaneHeaderTabStrip` gets
     *  its tab pills into the SAME row as those already-correct pieces,
     *  without reimplementing any of them. */
    leadingTabStrip?: JSX.Element;
}
