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
}
