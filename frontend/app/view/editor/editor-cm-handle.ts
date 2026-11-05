// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { EditorView } from "codemirror";

/** The pane's live CodeMirror view, shared by the pieces of the Editor view; null until one is built. */
export interface CmHandle {
    view: EditorView | null;
}
