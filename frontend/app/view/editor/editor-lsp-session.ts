// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, type Accessor } from "solid-js";
import type { Compartment } from "@codemirror/state";
import { settingsAtom } from "@/store/global";
import type { CmHandle } from "./editor-cm-handle";
import { decodeFileUri, workspaceCovers } from "./editor-lsp-paths";
import type { EditorViewModel } from "./editor-model";
import { isLspSupportedLanguage } from "./lsp/install-hints";
import { LspClient, type LspState } from "./lsp/lsp-client";
import { lspDiagnosticsExtension } from "./lsp/lsp-extensions";

/**
 * The language server side of one Editor pane (Phase 1 — diagnostics for TS/JS).
 * One LspClient per (pane, file), replaced when the file changes or the language
 * switches. Diagnostics flow through a Compartment so they can be reconfigured
 * without rebuilding CodeMirror.
 */
export class LspSession {
    private client: LspClient | null = null;
    private diagUnsub: (() => void) | null = null;
    private stateUnsub: (() => void) | null = null;
    private changeDebounce: ReturnType<typeof setTimeout> | null = null;
    private readonly state = createSignal<LspState | null>(null);
    readonly lspState: Accessor<LspState | null> = this.state[0];

    constructor(
        private readonly model: EditorViewModel,
        private readonly cm: CmHandle,
        private readonly lintCompartment: Compartment,
    ) {}

    async teardown(): Promise<void> {
        if (this.changeDebounce) {
            clearTimeout(this.changeDebounce);
            this.changeDebounce = null;
        }
        if (this.diagUnsub) {
            this.diagUnsub();
            this.diagUnsub = null;
        }
        if (this.stateUnsub) {
            this.stateUnsub();
            this.stateUnsub = null;
        }
        const old = this.client;
        this.client = null;
        this.state[1](null);
        if (old) {
            await old.dispose();
        }
    }

    /** Debounced didChange — Phase 1 ships full-sync, the simplest and works on every server. */
    scheduleChange(content: string): void {
        if (this.changeDebounce) clearTimeout(this.changeDebounce);
        this.changeDebounce = setTimeout(() => {
            if (this.client && this.client.getState().kind === "ready") {
                void this.client.didChange(this.model.filePathAtom(), content);
            }
        }, 250);
    }

    async startIfSupported(filePath: string, language: string, content: string): Promise<void> {
        // Bail early on an unsupported language or the master kill switch, but tear
        // down any client carried over from the previous file first: otherwise it
        // stays alive (and its server-side refcount), debounced didChange calls keep
        // firing, and the status chip and banner still reflect the old file.
        if (!isLspSupportedLanguage(language) || settingsAtom()?.["editor:lsp.enabled"] === false) {
            await this.teardown();
            return;
        }

        // Reuse the existing client when only the file changed (same language AND
        // the new file under the server's workspace root): the refcount-sharing
        // design relies on it, and tearing down per file would respawn the server on
        // every navigation.
        const existing = this.client;
        if (
            existing &&
            existing.language === language &&
            existing.getState().kind === "ready" &&
            workspaceCovers(existing.getWorkspaceRoot(), filePath)
        ) {
            await this.reuse(existing, filePath, language, content);
            return;
        }

        await this.teardown();

        const client = new LspClient(language, filePath);
        this.client = client;
        this.stateUnsub = client.onStateChange(this.state[1]);

        const ok = await client.start();
        if (!ok) {
            // Status is "missing" or "crashed" — banner takes over from here.
            return;
        }
        await client.didOpen(filePath, content, language);
        if (this.cm.view && this.client === client) {
            const [ext, unsub] = lspDiagnosticsExtension(this.cm.view, client);
            this.diagUnsub = unsub;
            this.cm.view.dispatch({ effects: this.lintCompartment.reconfigure(ext) });
        }
    }

    /** The same client takes the new file; its diagnostics are re-wired because setupEditor rebuilt the view. */
    private async reuse(existing: LspClient, filePath: string, language: string, content: string): Promise<void> {
        // Cancel any pending didChange from the OUTGOING file: the debounced callback
        // reads model.filePathAtom() when it fires, so a stale tick would send the
        // previous file's text under the new file's URI.
        if (this.changeDebounce) {
            clearTimeout(this.changeDebounce);
            this.changeDebounce = null;
        }
        const prevUri = existing.getOpenedFileUri();
        if (prevUri) {
            // Best-effort close of the previous file.
            void existing.didClose(decodeFileUri(prevUri));
        }
        await existing.didOpen(filePath, content, language);
        if (this.cm.view) {
            if (this.diagUnsub) this.diagUnsub();
            const [ext, unsub] = lspDiagnosticsExtension(this.cm.view, existing);
            this.diagUnsub = unsub;
            this.cm.view.dispatch({ effects: this.lintCompartment.reconfigure(ext) });
        }
    }
}
