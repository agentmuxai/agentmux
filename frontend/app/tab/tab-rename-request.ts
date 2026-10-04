// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Lets a keyboard shortcut start renaming a window tab, the same as
// double-clicking it. Each tab watches the request and reacts to its own id.

import { createSignal } from "solid-js";

const [renameRequest, setRenameRequest] = createSignal<{ tabId: string; seq: number } | null>(null);
let seq = 0;

export function requestTabRename(tabId: string): void {
    setRenameRequest({ tabId, seq: ++seq });
}

export { renameRequest };
