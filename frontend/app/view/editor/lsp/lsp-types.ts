// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Minimal LSP type subset — enough for Phase 1 (diagnostics) + the
// initialize/didOpen/didChange/didClose lifecycle. Expanded as later
// phases pull in completion, hover, definition.
//
// Spec: docs/specs/SPEC_EDITOR_LSP_AND_THEMES_2026-05-26.md

export interface LspPosition {
    line: number; // 0-based
    character: number; // 0-based UTF-16 code units (LSP convention)
}

export interface LspRange {
    start: LspPosition;
    end: LspPosition;
}

export type LspDiagnosticSeverity = 1 | 2 | 3 | 4; // Error | Warning | Information | Hint

export interface LspDiagnostic {
    range: LspRange;
    severity?: LspDiagnosticSeverity;
    code?: string | number;
    source?: string;
    message: string;
    tags?: number[];
}

export interface LspPublishDiagnosticsParams {
    uri: string;
    version?: number;
    diagnostics: LspDiagnostic[];
}

/** Wire envelope for a server-pushed message arriving via `lsp:message`. */
export interface LspMessageEnvelope {
    server_id: string;
    message: LspNotification | LspResponse;
}

export interface LspNotification {
    jsonrpc: "2.0";
    method: string;
    params?: unknown;
}

export interface LspResponse {
    jsonrpc: "2.0";
    id: number;
    result?: unknown;
    error?: { code: number; message: string; data?: unknown };
}

/** The LSP `initialize` request's response carries the server's
 *  capability declarations. We don't enforce them in Phase 1 — diagnostics
 *  are server-pushed regardless — but storing them lets later phases
 *  branch on what the server supports. */
export interface LspInitializeResult {
    capabilities: {
        textDocumentSync?: number | { openClose?: boolean; change?: number };
        completionProvider?: { triggerCharacters?: string[] };
        hoverProvider?: boolean | object;
        definitionProvider?: boolean | object;
        // ... more in later phases
    };
    serverInfo?: { name: string; version?: string };
}
