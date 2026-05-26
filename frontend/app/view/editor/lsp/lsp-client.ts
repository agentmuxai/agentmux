// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// LspClient — thin wrapper around the backend's `lspstart` / `lspsend` /
// `lspstop` RPCs. Maintains the LSP request/response cycle (assigns ids,
// resolves promises when matching responses arrive via the `lsp:message`
// WS event), and routes server-pushed notifications to subscribers.
//
// One LspClient per (editor pane, language) — created on first openFile,
// disposed on pane close or file-language change.
//
// Spec: specs/SPEC_EDITOR_LSP_AND_THEMES_2026-05-26.md

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import type {
    LspInitializeResult,
    LspMessageEnvelope,
    LspPublishDiagnosticsParams,
    LspResponse,
} from "./lsp-types";

export type LspState =
    | { kind: "starting" }
    | { kind: "initializing" }
    | { kind: "ready"; capabilities: LspInitializeResult["capabilities"] }
    | { kind: "missing"; language: string } // server binary not installed
    | { kind: "crashed"; error: string }
    | { kind: "disposed" };

type DiagnosticsHandler = (params: LspPublishDiagnosticsParams) => void;

export class LspClient {
    private nextRequestId = 1;
    private serverId: string | null = null;
    private workspaceRoot: string | null = null;
    private state: LspState = { kind: "starting" };
    private readonly pending = new Map<number, (response: LspResponse) => void>();
    private diagnosticsHandler: DiagnosticsHandler | null = null;
    private stateChangeHandlers = new Set<(s: LspState) => void>();
    private unsubscribeWs: (() => void) | null = null;
    private fileVersion = 0;
    private openedFileUri: string | null = null;

    constructor(
        readonly language: string,
        readonly filePath: string,
    ) {}

    /** Start the server + send `initialize` + `initialized`. Resolves
     *  to `true` on success, `false` if the binary's missing (banner case). */
    async start(): Promise<boolean> {
        try {
            const result = await RpcApi.LspStartCommand(TabRpcClient, {
                language: this.language,
                file_path: this.filePath,
            });
            this.serverId = result.server_id;
            this.workspaceRoot = result.workspace_root;
        } catch (e: any) {
            const msg = e?.message ?? String(e);
            if (msg.includes("server_binary_not_found")) {
                this.setState({ kind: "missing", language: this.language });
                return false;
            }
            this.setState({ kind: "crashed", error: msg });
            return false;
        }

        // Subscribe to server-pushed messages BEFORE sending initialize so we
        // don't miss the initialize response.
        this.subscribe();

        this.setState({ kind: "initializing" });
        try {
            const initResult = (await this.request("initialize", {
                processId: null,
                clientInfo: { name: "agentmux-editor", version: "1" },
                rootUri: this.workspaceRoot ? pathToFileUri(this.workspaceRoot) : null,
                capabilities: {
                    textDocument: {
                        synchronization: { didSave: false, dynamicRegistration: false },
                        publishDiagnostics: { relatedInformation: true },
                    },
                },
            })) as LspInitializeResult;

            // Notify `initialized` (no response expected)
            this.notify("initialized", {});

            this.setState({ kind: "ready", capabilities: initResult.capabilities });
            return true;
        } catch (e: any) {
            this.setState({ kind: "crashed", error: e?.message ?? String(e) });
            return false;
        }
    }

    /** Tell the server we just opened a file. */
    async didOpen(filePath: string, content: string, languageId: string): Promise<void> {
        if (this.state.kind !== "ready") return;
        const uri = pathToFileUri(filePath);
        this.openedFileUri = uri;
        this.fileVersion = 1;
        this.notify("textDocument/didOpen", {
            textDocument: { uri, languageId, version: this.fileVersion, text: content },
        });
    }

    /** Send a full-document change. (Phase 1 is full-sync only; incremental
     *  comes with the completion/hover phase when text-document sync flag matters.) */
    async didChange(filePath: string, content: string): Promise<void> {
        if (this.state.kind !== "ready") return;
        this.fileVersion += 1;
        this.notify("textDocument/didChange", {
            textDocument: { uri: pathToFileUri(filePath), version: this.fileVersion },
            contentChanges: [{ text: content }],
        });
    }

    /** Close a file. */
    async didClose(filePath: string): Promise<void> {
        if (this.state.kind !== "ready") return;
        this.notify("textDocument/didClose", {
            textDocument: { uri: pathToFileUri(filePath) },
        });
        if (this.openedFileUri === pathToFileUri(filePath)) {
            this.openedFileUri = null;
        }
    }

    /** Wire a callback for publishDiagnostics. */
    onDiagnostics(handler: DiagnosticsHandler): () => void {
        this.diagnosticsHandler = handler;
        return () => {
            if (this.diagnosticsHandler === handler) this.diagnosticsHandler = null;
        };
    }

    /** Subscribe to LspClient state changes. */
    onStateChange(handler: (s: LspState) => void): () => void {
        this.stateChangeHandlers.add(handler);
        handler(this.state); // fire current
        return () => {
            this.stateChangeHandlers.delete(handler);
        };
    }

    getState(): LspState {
        return this.state;
    }

    /** Workspace root the server is anchored to (set after start()). */
    getWorkspaceRoot(): string | null {
        return this.workspaceRoot;
    }

    /** URI currently registered via didOpen, or null. */
    getOpenedFileUri(): string | null {
        return this.openedFileUri;
    }

    /** Tear down — send shutdown to server (best-effort) and call lspstop. */
    async dispose(): Promise<void> {
        if (this.state.kind === "disposed") return;
        if (this.state.kind === "ready") {
            // Best-effort graceful shutdown per LSP spec:
            //   1. textDocument/didClose for the open doc (notification)
            //   2. shutdown (request, awaited briefly — server flushes state)
            //   3. exit (notification — server self-terminates)
            // We bound the shutdown wait so a hung server can't block teardown;
            // lspstop on the backend kills the child via kill_on_drop regardless.
            try {
                if (this.openedFileUri) {
                    this.notify("textDocument/didClose", {
                        textDocument: { uri: this.openedFileUri },
                    });
                }
                const shutdownAck = this.request("shutdown", null);
                await Promise.race([
                    shutdownAck.catch(() => undefined),
                    new Promise((r) => setTimeout(r, 500)),
                ]);
                this.notify("exit", null);
            } catch {
                // ignore
            }
        }
        if (this.serverId) {
            try {
                await RpcApi.LspStopCommand(TabRpcClient, { server_id: this.serverId });
            } catch {
                // ignore
            }
        }
        if (this.unsubscribeWs) {
            this.unsubscribeWs();
            this.unsubscribeWs = null;
        }
        this.setState({ kind: "disposed" });
    }

    // ── Internals ──────────────────────────────────────────────────────

    private setState(state: LspState): void {
        this.state = state;
        for (const h of this.stateChangeHandlers) h(state);
    }

    /** JSON-RPC request — assigns an id, returns a promise that resolves
     *  when the matching response arrives. */
    private request(method: string, params: unknown): Promise<unknown> {
        if (!this.serverId) return Promise.reject(new Error("LSP not started"));
        const id = this.nextRequestId++;
        const message = { jsonrpc: "2.0", id, method, params };
        return new Promise((resolve, reject) => {
            this.pending.set(id, (response) => {
                if (response.error) reject(new Error(response.error.message));
                else resolve(response.result);
            });
            void RpcApi.LspSendCommand(TabRpcClient, {
                server_id: this.serverId as string,
                message,
            }).catch((e) => {
                this.pending.delete(id);
                reject(e);
            });
        });
    }

    /** JSON-RPC notification — fire-and-forget (no id). */
    private notify(method: string, params: unknown): void {
        if (!this.serverId) return;
        void RpcApi.LspSendCommand(TabRpcClient, {
            server_id: this.serverId,
            message: { jsonrpc: "2.0", method, params },
        });
    }

    private subscribe(): void {
        if (this.unsubscribeWs) return;
        this.unsubscribeWs = waveEventSubscribe({
            eventType: "lsp:message",
            handler: (event) => {
                const envelope = event.data as LspMessageEnvelope | undefined;
                if (!envelope || envelope.server_id !== this.serverId) return;
                const msg = envelope.message;
                if ("id" in msg && typeof msg.id === "number") {
                    // Response to one of our requests
                    const resolver = this.pending.get(msg.id);
                    if (resolver) {
                        this.pending.delete(msg.id);
                        resolver(msg as LspResponse);
                    }
                    return;
                }
                if ("method" in msg) {
                    // Server-pushed notification
                    this.handleNotification(msg.method, msg.params);
                }
            },
        });
    }

    private handleNotification(method: string, params: unknown): void {
        if (method === "textDocument/publishDiagnostics") {
            const p = params as LspPublishDiagnosticsParams;
            if (this.diagnosticsHandler && p?.uri === this.openedFileUri) {
                this.diagnosticsHandler(p);
            }
            return;
        }
        // $/progress, window/logMessage, etc. — ignore in Phase 1.
    }
}

/** Convert an OS file path to a file:// URI. LSP requires this for
 *  every document identifier. Windows paths need extra care: backslashes
 *  → forward slashes, drive letter prefixed with /.  */
function pathToFileUri(p: string): string {
    if (/^[a-zA-Z]:[\\/]/.test(p)) {
        // Windows absolute path
        const normalized = p.replace(/\\/g, "/");
        return "file:///" + normalized;
    }
    // POSIX absolute
    return "file://" + p;
}
