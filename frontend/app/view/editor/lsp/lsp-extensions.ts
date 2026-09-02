// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CodeMirror 6 extension factories that wire an LspClient to the editor.
// Phase 1 ships ONE: diagnostics (LSP publishDiagnostics → CM6 lint markers).
// Completion/hover/definition land in Phase 2.
//
// Spec: docs/specs/SPEC_EDITOR_LSP_AND_THEMES_2026-05-26.md

import { Diagnostic, setDiagnostics } from "@codemirror/lint";
import type { EditorView } from "@codemirror/view";
import type { Extension } from "@codemirror/state";
import type { LspClient } from "./lsp-client";
import type { LspDiagnosticSeverity } from "./lsp-types";

/** Map LSP diagnostic severity to CM6 lint severity. */
function severityToCm(s?: LspDiagnosticSeverity): Diagnostic["severity"] {
    switch (s) {
        case 1:
            return "error";
        case 2:
            return "warning";
        case 3:
            return "info";
        case 4:
            return "info"; // CM6 has no "hint" tier; collapse to info
        default:
            return "info";
    }
}

/**
 * Returns an Extension that subscribes to the client's diagnostics and
 * pushes them into the editor as lint markers. The caller is responsible
 * for disposing the returned unsubscribe (returned via the second
 * element of the tuple) when the editor/file unmounts.
 *
 * Use:
 *   const [ext, unsub] = lspDiagnosticsExtension(view, client);
 *   editor.dispatch({ effects: someCompartment.reconfigure(ext) });
 *   onCleanup(unsub);
 */
export function lspDiagnosticsExtension(
    view: EditorView,
    client: LspClient,
): [Extension, () => void] {
    // The LSP server pushes diagnostics asynchronously. We translate each
    // diagnostic's LSP range to a CM6 document offset and call
    // `setDiagnostics` directly on the view — no per-diagnostic state in
    // the extension itself, so the extension is essentially empty.
    const unsub = client.onDiagnostics((params) => {
        const doc = view.state.doc;
        const cmDiagnostics: Diagnostic[] = params.diagnostics.map((d) => {
            const from = lspPositionToOffset(doc, d.range.start);
            const to = lspPositionToOffset(doc, d.range.end);
            return {
                from,
                to: Math.max(to, from + 1), // CM6 zero-length diagnostics swallow visually
                severity: severityToCm(d.severity),
                message: d.source ? `[${d.source}] ${d.message}` : d.message,
                source: typeof d.code === "string" || typeof d.code === "number" ? String(d.code) : undefined,
            };
        });
        view.dispatch(setDiagnostics(view.state, cmDiagnostics));
    });

    // The extension itself is a no-op marker — the work happens via the
    // subscription. Returning `[]` (empty extension) keeps the API clean.
    return [[], unsub];
}

/** Convert an LSP {line, character} position to a CM6 absolute offset.
 *  LSP `character` is UTF-16 code units, matching JS string indexing. */
function lspPositionToOffset(doc: import("@codemirror/state").Text, pos: { line: number; character: number }): number {
    const lineNum = Math.max(0, Math.min(doc.lines - 1, pos.line));
    const line = doc.line(lineNum + 1); // CM6 lines are 1-based
    const ch = Math.max(0, Math.min(line.length, pos.character));
    return line.from + ch;
}
