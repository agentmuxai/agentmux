// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Memory pane view — thin wrapper around the context-free
// <MemoryManagerBody/>.
//
// The full list / create / edit / delete UI was extracted into
// `memory-manager.tsx` (PR 2 of SPEC_BUNDLE_MANAGEMENT_2026_05_22.md)
// so the same surface can render both here (the `view: "memory"`
// settings pane) and inside the window-scoped bundle manager modal.
// This file exists only so the BlockRegistry barrel has a
// `viewComponent` taking the pane's ViewModel; it renders the shared
// body verbatim, so the settings pane is byte-for-byte unchanged.

import { type JSX } from "solid-js";

import { MemoryManagerBody } from "./memory-manager";
import type { MemoryViewModel } from "./memory-model";

interface MemoryViewProps {
    model: MemoryViewModel;
}

export const MemoryView = (props: MemoryViewProps): JSX.Element => {
    return <MemoryManagerBody model={props.model} />;
};
