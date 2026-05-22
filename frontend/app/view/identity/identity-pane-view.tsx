// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Identity pane view — thin wrapper around the context-free
// <IdentityManagerBody/>.
//
// The full list / create / edit / delete / bindings UI was extracted
// into `identity-manager.tsx` (PR 2 of
// SPEC_BUNDLE_MANAGEMENT_2026_05_22.md) so the same surface can render
// both here (the `view: "identity"` settings pane) and inside the
// window-scoped bundle manager modal. This file exists only so the
// BlockRegistry barrel has a `viewComponent` taking the pane's
// ViewModel; it renders the shared body verbatim, so the settings pane
// is byte-for-byte unchanged.

import { type JSX } from "solid-js";

import { IdentityManagerBody } from "./identity-manager";
import type { IdentityPaneViewModel } from "./identity-pane-model";

interface IdentityPaneViewProps {
    model: IdentityPaneViewModel;
}

export const IdentityPaneView = (props: IdentityPaneViewProps): JSX.Element => {
    return <IdentityManagerBody model={props.model} />;
};
