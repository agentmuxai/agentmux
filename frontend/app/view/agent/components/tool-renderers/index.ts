// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Every tool-result renderer, in one explicit, ordered list
 * (SPEC_AGENT_PANE_TOOL_DESCRIPTORS_2026_09_26.md §2.5).
 *
 * Renderers used to register themselves as an import side effect, so a new
 * one was silently dead until someone remembered its bare import, and the
 * winner among equal priorities depended on import order. Now the list is
 * the registry's source of truth; `registerToolRenderers()` is idempotent (the
 * registry replaces by label), so HMR and a second call are harmless.
 */

import { BUILTIN_RENDERERS } from "./builtins";
import { dispatchCardRenderer } from "./DispatchCard";
import { recordTableRenderer } from "./RecordTable";
import { registerToolRenderer, type ToolRendererEntry } from "./registry";
import { searchResultsRenderer } from "./SearchResults";
import { toolReferencesRenderer } from "./ToolReferences";
import { webFetchRenderer } from "./WebFetchResult";

// Same order the side-effect imports used to produce: the rich renderers
// first, then the built-ins. Ties only break by order among equal priorities.
export const TOOL_RENDERERS: readonly ToolRendererEntry[] = [
    searchResultsRenderer,
    webFetchRenderer,
    recordTableRenderer,
    dispatchCardRenderer,
    toolReferencesRenderer,
    ...BUILTIN_RENDERERS,
];

export function registerToolRenderers(): void {
    for (const entry of TOOL_RENDERERS) registerToolRenderer(entry);
}
