// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentInstallModalPanel — modal that runs an agent's install recipe
 * and shows live output. Opens when the user picks an agent whose CLI
 * isn't already in the per-version cache. Sibling to
 * `AgentLaunchModalPanel`.
 *
 * Phase α (SPEC_AGENT_INSTALL_STAGE_2026_05_17.md §11): single-step
 * recipe (just `npm install <package>`) streamed line-by-line via the
 * `install.start` RPC. The visual surface is a scrollable `<pre>` for
 * v1 — xterm.js upgrade lands in Phase β. Cancel kills the install +
 * removes the partial dir.
 *
 * Re-click idempotency: the modal owns the install lifecycle. While a
 * session is in flight, opening this modal again (re-click on the
 * picker card) re-focuses the existing modal instead of starting a
 * second install.
 */

import { Show, createSignal, onCleanup, onMount, type JSX } from "solid-js";

import { Button } from "@/element/button";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";

import { getCliCatalogEntry } from "../defaults/cli-catalog";
import { getProvider } from "../providers";

interface AgentInstallModalPanelProps {
    agent: ForgeAgent;
    onCancel: () => void;
    onInstalled: () => void;
}

interface InstallChunk {
    line: string;
    stream: "stdout" | "stderr";
}

export const AgentInstallModalPanel = (props: AgentInstallModalPanelProps): JSX.Element => {
    const catalog = () => getCliCatalogEntry(props.agent.provider);
    const provider = () => getProvider(props.agent.provider);
    const displayName = () => catalog()?.displayName ?? props.agent.name;

    const [phase, setPhase] = createSignal<"idle" | "installing" | "done" | "failed">("idle");
    const [chunks, setChunks] = createSignal<InstallChunk[]>([]);
    const [error, setError] = createSignal<string | null>(null);
    const [sessionId, setSessionId] = createSignal<string | null>(null);
    const [elapsedMs, setElapsedMs] = createSignal(0);

    let unsub: (() => void) | null = null;
    let preRef: HTMLPreElement | undefined;
    let startedAt = 0;
    let tickHandle: ReturnType<typeof setInterval> | null = null;
    // Flipped in onCleanup so a startInstall awaiting the RPC response
    // can cancel the resolved session id even if it landed after unmount.
    let disposed = false;

    const appendChunk = (chunk: InstallChunk) => {
        setChunks((prev) => {
            const next = prev.concat(chunk);
            // Cap scrollback so very chatty installs don't drag the
            // renderer. 2000 lines is enough for typical npm output;
            // last-N is what the user needs anyway.
            if (next.length > 2000) return next.slice(next.length - 2000);
            return next;
        });
        // Auto-scroll to bottom.
        queueMicrotask(() => {
            if (preRef) preRef.scrollTop = preRef.scrollHeight;
        });
    };

    const startInstall = async () => {
        const prov = provider();
        if (!prov) {
            setError(`unknown provider ${props.agent.provider}`);
            setPhase("failed");
            return;
        }
        // Tear down any prior run (Retry path) before reassigning —
        // otherwise the previous setInterval keeps ticking + the
        // previous WPS subscription stays active for the rest of the
        // component's life.
        if (unsub) {
            unsub();
            unsub = null;
        }
        if (tickHandle != null) {
            clearInterval(tickHandle);
            tickHandle = null;
        }
        setPhase("installing");
        setChunks([]);
        setError(null);
        startedAt = Date.now();
        tickHandle = setInterval(() => setElapsedMs(Date.now() - startedAt), 250);
        try {
            const r = await RpcApi.InstallStartCommand(TabRpcClient, {
                providerId: prov.id,
                cliCommand: prov.cliCommand,
                npmPackage: prov.npmPackage,
                pinnedVersion: prov.pinnedVersion,
            });
            // If the modal unmounted while the RPC was in flight, the
            // backend now owns a live session our cleanup couldn't
            // cancel (sessionId was null). Cancel it explicitly here.
            if (disposed) {
                void RpcApi.InstallCancelCommand(TabRpcClient, { sessionId: r.sessionId }).catch(() => {
                    /* best-effort */
                });
                return;
            }
            setSessionId(r.sessionId);
            unsub = waveEventSubscribe({
                eventType: "install_chunk",
                scope: `install:${r.sessionId}`,
                handler: (event: any) => {
                    const data = event?.data;
                    if (!data || typeof data !== "object") return;
                    if (typeof data.line === "string") {
                        appendChunk({
                            line: data.line,
                            stream: data.stream === "stderr" ? "stderr" : "stdout",
                        });
                    } else if (data.op === "done") {
                        if (data.ok) {
                            setPhase("done");
                            // Auto-close + chain to launch modal one
                            // frame later so the user sees the "done"
                            // state briefly.
                            queueMicrotask(() => props.onInstalled());
                        } else {
                            setError(data.error ?? "install failed");
                            setPhase("failed");
                        }
                    }
                },
            });
        } catch (e) {
            setError((e as Error)?.message ?? String(e));
            setPhase("failed");
        }
    };

    const cancel = async () => {
        const sid = sessionId();
        if (sid) {
            try {
                await RpcApi.InstallCancelCommand(TabRpcClient, { sessionId: sid });
            } catch {
                /* ignore — best-effort */
            }
        }
        props.onCancel();
    };

    onMount(() => {
        void startInstall();
    });

    onCleanup(() => {
        disposed = true;
        if (unsub) {
            unsub();
            unsub = null;
        }
        if (tickHandle != null) {
            clearInterval(tickHandle);
            tickHandle = null;
        }
        // Catch implicit closes (Esc, backdrop click, tab switch)
        // while installing. The Cancel button itself goes through
        // `cancel()`. If the modal unmounts *before* InstallStartCommand
        // resolves, the `disposed` flag above lets the in-flight start
        // path issue the cancel once the session id arrives.
        const sid = sessionId();
        if (sid && phase() === "installing") {
            void RpcApi.InstallCancelCommand(TabRpcClient, { sessionId: sid }).catch(() => {
                /* best-effort */
            });
        }
    });

    const elapsedLabel = () => {
        const s = Math.floor(elapsedMs() / 1000);
        const mm = Math.floor(s / 60).toString().padStart(1, "0");
        const ss = (s % 60).toString().padStart(2, "0");
        return `${mm}:${ss}`;
    };

    return (
        <div class="agent-install-modal-panel">
            <div class="agent-install-modal-header">
                <span class="agent-install-modal-icon" aria-hidden="true">
                    {catalog()?.icon ?? "📦"}
                </span>
                <span class="agent-install-modal-title">Install {displayName()}</span>
                <span class="agent-install-modal-status">
                    <Show when={phase() === "installing"}>
                        <span class="agent-install-modal-spinner">⏳</span>
                        Installing… {elapsedLabel()}
                    </Show>
                    <Show when={phase() === "done"}>
                        <span class="agent-install-modal-ok">✓</span>
                        Installed
                    </Show>
                    <Show when={phase() === "failed"}>
                        <span class="agent-install-modal-fail">✗</span>
                        Failed
                    </Show>
                </span>
            </div>
            <pre class="agent-install-modal-log" ref={preRef}>
                {chunks().map((c) => (
                    <span
                        classList={{
                            "agent-install-modal-line": true,
                            "agent-install-modal-line--stderr": c.stream === "stderr",
                        }}
                    >
                        {c.line}
                        {"\n"}
                    </span>
                ))}
            </pre>
            <Show when={error()}>
                <div class="agent-install-modal-error">⚠ {error()}</div>
            </Show>
            <div class="agent-install-modal-actions">
                <Show when={phase() === "installing"}>
                    <Button onClick={() => void cancel()}>Cancel</Button>
                </Show>
                <Show when={phase() === "failed"}>
                    <Button onClick={() => void startInstall()} className="green solid">
                        Retry
                    </Button>
                    <Button onClick={() => props.onCancel()}>Close</Button>
                </Show>
                <Show when={phase() === "done"}>
                    <span class="agent-install-modal-launching">Opening launch modal…</span>
                </Show>
            </div>
        </div>
    );
};

AgentInstallModalPanel.displayName = "AgentInstallModalPanel";
