// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether the pane-stack member a component lives in is its stack's active
 * one. Provided per keep-alive slot by `pane-leaf-chrome.tsx` (combined with
 * any outer stack), true outside any stack. The slot's own `data-pane-dormant`
 * marker says the same in the DOM; this reaches what a Portal mounts under
 * `document.body`, where no marker is above it (pane-overlay.ts).
 */
import { createContext, useContext, type Accessor } from "solid-js";

const PaneStackMemberActiveContext = createContext<Accessor<boolean>>(() => true);

export const PaneStackMemberActiveProvider = PaneStackMemberActiveContext.Provider;

export function usePaneStackMemberActive(): Accessor<boolean> {
    return useContext(PaneStackMemberActiveContext);
}
