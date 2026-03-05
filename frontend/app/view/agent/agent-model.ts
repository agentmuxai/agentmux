// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { getFileSubject, waveEventSubscribe } from "@/app/store/wps";
import { atoms, globalStore, WOS, getApi } from "@/app/store/global";
import * as services from "@/app/store/services";
import { base64ToArray, stringToBase64 } from "@/util/util";
import { atom, Atom, PrimitiveAtom } from "jotai";
import React from "react";
import { Subscription } from "rxjs";
import { AgentViewWrapper } from "./agent-view";
import { ClaudeCodeStreamParser } from "./stream-parser";
import {
    AgentAtoms,
    createAgentAtoms,
    createFilteredDocumentAtom,
    createDocumentStatsAtom,
    createToggleNodeCollapsed,
} from "./state";
import { PROVIDERS, type ProviderDefinition } from "./providers";
import { createTranslator } from "./providers/translator-factory";
import type { OutputTranslator } from "./providers/translator";
import type { DocumentNode } from "./types";

const TermFileName = "term";

export class AgentViewModel implements ViewModel {
    viewType = "agent";
    blockId: string;
    nodeModel: BlockNodeModel;
    blockAtom: Atom<Block>;

    viewIcon: Atom<string>;
    viewName: Atom<string>;
    viewText: Atom<string | HeaderElem[]>;
    viewComponent: ViewComponent;
    noPadding = atom(true);

    agentIdValue: string;
    atoms: AgentAtoms;
    filteredDocumentAtom: Atom<DocumentNode[]>;
    documentStatsAtom: Atom<any>;
    toggleNodeCollapsed: any;

    inputRef: React.RefObject<HTMLTextAreaElement>;
    shellProcStatusAtom: Atom<string>;
    shellProcExitCodeAtom: Atom<number>;

    // Internal
    private provider: ProviderDefinition | null = null;
    private cliPath: string | null = null;
    private parser: ClaudeCodeStreamParser;
    private translator: OutputTranslator | null = null;
    private fileSubjectSub: Subscription | null = null;
    private fileSubjectRef: any = null;
    private procStatusUnsub: (() => void) | null = null;
    private shellProcFullStatusAtom: PrimitiveAtom<BlockControllerRuntimeStatus>;
    private loginMode: boolean = false;
    private loginUrlOpened: boolean = false;
    private viewIconAtom: PrimitiveAtom<string>;
    private viewNameAtom: PrimitiveAtom<string>;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        this.blockAtom = WOS.getWaveObjectAtom<Block>(`block:${blockId}`);
        this.viewComponent = AgentViewWrapper as any;

        this.agentIdValue = blockId;
        this.atoms = createAgentAtoms(blockId);

        this.filteredDocumentAtom = createFilteredDocumentAtom(
            this.atoms.documentAtom,
            this.atoms.documentStateAtom
        );
        this.documentStatsAtom = createDocumentStatsAtom(this.atoms.documentAtom);
        this.toggleNodeCollapsed = createToggleNodeCollapsed(this.atoms.documentStateAtom);

        this.viewIconAtom = atom("sparkles") as PrimitiveAtom<string>;
        this.viewNameAtom = atom("Agent") as PrimitiveAtom<string>;
        this.viewIcon = this.viewIconAtom;
        this.viewName = this.viewNameAtom;
        this.viewText = atom((get) => {
            const stats = get(this.documentStatsAtom);
            const parts: HeaderElem[] = [];
            if (stats.totalNodes > 0) {
                parts.push({ elemtype: "text", text: `${stats.totalNodes} events` });
            }
            return parts;
        });

        this.inputRef = React.createRef<HTMLTextAreaElement>();
        this.shellProcFullStatusAtom = atom(null) as unknown as PrimitiveAtom<BlockControllerRuntimeStatus>;
        this.shellProcStatusAtom = atom((get) => {
            const fullStatus = get(this.shellProcFullStatusAtom);
            return fullStatus?.shellprocstatus ?? "init";
        });
        this.shellProcExitCodeAtom = atom((get) => {
            const fullStatus = get(this.shellProcFullStatusAtom);
            return fullStatus?.shellprocexitcode ?? 0;
        });

        // Subscribe to controller status events
        this.procStatusUnsub = waveEventSubscribe({
            eventType: "controllerstatus",
            scope: WOS.makeORef("block", blockId),
            handler: (event) => {
                const rts = event.data as BlockControllerRuntimeStatus;
                this.updateShellProcStatus(rts);
            },
        });

        this.parser = new ClaudeCodeStreamParser();
    }

    // =============================================
    // Connect with provider: the new entry point
    // =============================================

    /**
     * Called when user clicks a provider button.
     * Installs CLI if needed, then checks auth and starts session.
     */
    connectWithProvider = async (providerId: string, cliPath: string): Promise<void> => {
        const provider = PROVIDERS[providerId];
        if (!provider) {
            console.error("[agent] Unknown provider:", providerId);
            return;
        }

        this.provider = provider;
        this.cliPath = cliPath;

        // Update header to show provider
        globalStore.set(this.viewIconAtom, provider.icon);
        globalStore.set(this.viewNameAtom, provider.displayName);

        // Create translator only if not raw mode
        if (provider.outputFormat !== "raw") {
            this.translator = createTranslator(provider.outputFormat);
        } else {
            this.translator = null;
        }

        globalStore.set(this.atoms.authAtom, { status: "connecting" });
        console.log(`[agent] Checking ${providerId} auth status (cli: ${cliPath})...`);

        try {
            const authStatus = await getApi().checkCliAuthStatus(providerId, cliPath);
            console.log("[agent] Auth result:", authStatus);

            if (authStatus.logged_in) {
                globalStore.set(this.atoms.authAtom, { status: "connected" });
                globalStore.set(this.atoms.userInfoAtom, {
                    email: authStatus.email || "",
                    name: authStatus.subscription_type || "",
                });
                this.startSession();
            } else {
                this.startAuthLogin();
            }
        } catch (error) {
            console.error("[agent] Auth check failed:", error);
            globalStore.set(this.atoms.authAtom, {
                status: "error",
                error: `Auth check failed: ${String(error)}`,
            });
        }
    };

    /**
     * Legacy connect method — kept for backward compat but now requires
     * connectWithProvider to be called instead.
     */
    connect = async (): Promise<void> => {
        // No-op: the 3-button UI calls connectWithProvider directly
        console.warn("[agent] connect() called without provider — use connectWithProvider()");
    };

    /**
     * Spawn auth login in the PTY so user can authenticate via browser.
     * When the process exits, we re-check auth.
     */
    private startAuthLogin(): void {
        if (!this.provider || !this.cliPath) return;

        console.log(`[agent] Starting ${this.provider.id} auth login...`);
        this.loginMode = true;
        globalStore.set(this.atoms.authAtom, { status: "connecting" });

        const oref = WOS.makeORef("block", this.blockId);
        RpcApi.SetMetaCommand(TabRpcClient, {
            oref,
            meta: {
                "cmd": this.cliPath,
                "cmd:args": this.provider.authLoginCommand,
            },
        }).then(() => {
            this.connectToTerminal();
            return RpcApi.ControllerResyncCommand(TabRpcClient, {
                tabid: globalStore.get(atoms.staticTabId),
                blockid: this.blockId,
                forcerestart: true,
            });
        }).catch((e: any) => {
            console.error("[agent] Failed to start auth login:", e);
            globalStore.set(this.atoms.authAtom, {
                status: "error",
                error: `Login failed: ${String(e)}`,
            });
        });
    }

    /**
     * Start the CLI session. In raw mode, just launch the CLI with defaultArgs.
     */
    private startSession(): void {
        if (!this.provider || !this.cliPath) return;

        this.loginMode = false;
        console.log(`[agent] Starting ${this.provider.id} session (raw mode)...`);

        const oref = WOS.makeORef("block", this.blockId);
        RpcApi.SetMetaCommand(TabRpcClient, {
            oref,
            meta: {
                "cmd": this.cliPath,
                "cmd:args": this.provider.defaultArgs,
            },
        }).then(() => {
            this.connectToTerminal();
            return RpcApi.ControllerResyncCommand(TabRpcClient, {
                tabid: globalStore.get(atoms.staticTabId),
                blockid: this.blockId,
                forcerestart: true,
            });
        }).catch((e: any) => {
            console.error("[agent] Failed to start session:", e);
        });
    }

    // --- Terminal connection ---

    private connectToTerminal(): void {
        if (this.fileSubjectSub) return;

        try {
            this.fileSubjectRef = getFileSubject(this.blockId, TermFileName);
            this.fileSubjectSub = this.fileSubjectRef.subscribe((msg: any) => {
                this.handleTerminalData(msg);
            });
        } catch (e) {
            console.error("[agent] Failed to connect to terminal:", e);
        }
    }

    private disconnectTerminal(): void {
        if (this.fileSubjectSub) {
            this.fileSubjectSub.unsubscribe();
            this.fileSubjectSub = null;
        }
        if (this.fileSubjectRef) {
            this.fileSubjectRef.release();
            this.fileSubjectRef = null;
        }
    }

    private async handleTerminalData(msg: any): Promise<void> {
        if (msg.fileop === "truncate") {
            this.parser.reset();
            if (this.translator) this.translator.reset();
            return;
        }
        if (msg.fileop === "append" && msg.data64) {
            const bytes = base64ToArray(msg.data64);
            const text = new TextDecoder().decode(bytes);

            if (this.loginMode) {
                // Scan for OAuth URL in PTY output and open it in the browser.
                // Strip ANSI escape codes before searching.
                if (!this.loginUrlOpened) {
                    const plain = text.replace(/\x1b\[[0-9;]*[a-zA-Z]/g, "").replace(/[\x00-\x09\x0b-\x1f\x7f]/g, "");
                    const match = plain.match(/(https?:\/\/[^\s"'<>]+)/);
                    if (match) {
                        const url = match[1];
                        console.log("[agent] Opening OAuth URL:", url);
                        this.loginUrlOpened = true;
                        globalStore.set(this.atoms.authAtom, { status: "awaiting_browser" });
                        getApi().openExternal(url);
                    }
                }
                return;
            }

            // Raw mode: strip ANSI and append to rawOutputAtom
            if (!this.translator) {
                const plain = text.replace(/\x1b\[[0-9;]*[a-zA-Z]/g, "");
                const current = globalStore.get(this.atoms.rawOutputAtom);
                globalStore.set(this.atoms.rawOutputAtom, current + plain);
                return;
            }

            // Structured mode: parse NDJSON stream
            const lines = text.split('\n').filter(line => line.trim());
            for (const line of lines) {
                try {
                    const rawEvent = JSON.parse(line);
                    this.handleCliEvent(rawEvent);
                } catch {
                    // Non-JSON line, ignore
                }
            }
        }
    }

    /**
     * Route parsed CLI events by type.
     */
    private async handleCliEvent(event: any): Promise<void> {
        switch (event.type) {
            case "system":
                if (event.subtype === "init") {
                    console.log("[agent] Session init:", {
                        session_id: event.session_id,
                        model: event.model,
                    });
                    globalStore.set(this.atoms.sessionIdAtom, event.session_id || "");
                    globalStore.set(this.atoms.authAtom, { status: "connected" });
                }
                break;

            case "stream_event":
            case "assistant":
            case "user": {
                if (!this.translator) break;
                const streamEvents = this.translator.translate(event);
                for (const se of streamEvents) {
                    const nodes = await this.parser.parseEvent(se);
                    if (nodes.length > 0) {
                        const currentDoc = globalStore.get(this.atoms.documentAtom);
                        globalStore.set(this.atoms.documentAtom, [...currentDoc, ...nodes]);
                    }
                }
                break;
            }

            case "result":
                console.log("[agent] Session result:", {
                    is_error: event.is_error,
                    cost: event.total_cost_usd,
                });
                break;
        }
    }

    // --- User actions ---

    sendMessage = async (text: string): Promise<void> => {
        if (!text.trim()) return;
        const b64data = stringToBase64(text + "\n");
        RpcApi.ControllerInputCommand(TabRpcClient, {
            blockid: this.blockId,
            inputdata64: b64data,
        }).catch((e: any) => {
            console.error("[agent] Failed to send input:", e);
        });
    };

    killAgent = (): void => {
        RpcApi.ControllerInputCommand(TabRpcClient, {
            blockid: this.blockId,
            signame: "SIGINT",
        }).catch((e: any) => {
            console.error("[agent] Failed to send SIGINT:", e);
        });
    };

    restartAgent = (): void => {
        this.loginMode = false;
        this.loginUrlOpened = false;
        this.provider = null;
        this.cliPath = null;
        this.translator = null;
        globalStore.set(this.atoms.documentAtom, []);
        globalStore.set(this.atoms.rawOutputAtom, "");
        globalStore.set(this.atoms.authAtom, { status: "disconnected" });
        globalStore.set(this.viewIconAtom, "sparkles");
        globalStore.set(this.viewNameAtom, "Agent");
        this.disconnectTerminal();
    };

    private updateShellProcStatus(rts: BlockControllerRuntimeStatus): void {
        if (rts == null) return;
        const cur = globalStore.get(this.shellProcFullStatusAtom);
        if (cur == null || cur.version < rts.version) {
            globalStore.set(this.shellProcFullStatusAtom, rts);

            // When login process finishes, re-check auth
            if (this.loginMode && rts.shellprocstatus === "done") {
                console.log("[agent] Login process exited, code:", rts.shellprocexitcode);
                this.loginMode = false;
                this.loginUrlOpened = false;
                this.disconnectTerminal();
                // Re-check auth after login
                if (this.provider && this.cliPath) {
                    this.connectWithProvider(this.provider.id, this.cliPath);
                }
            }
        }
    }

    giveFocus(): boolean {
        if (this.inputRef.current) {
            requestAnimationFrame(() => this.inputRef.current?.focus());
            return true;
        }
        return false;
    }

    dispose(): void {
        this.disconnectTerminal();
        if (this.procStatusUnsub) {
            this.procStatusUnsub();
            this.procStatusUnsub = null;
        }
        this.parser.reset();
        if (this.translator) this.translator.reset();
    }
}
