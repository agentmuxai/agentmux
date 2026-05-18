// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentInstallModalPanel — modal that runs an agent's install recipe
 * and shows live output in an xterm.js terminal. Opens when the user
 * picks an agent whose CLI isn't already in the per-version cache.
 * Sibling to `AgentLaunchModalPanel`.
 *
 * Phase α (SPEC_AGENT_INSTALL_STAGE_2026_05_17.md §11): single-step
 * recipe (just `npm install <package>`) streamed line-by-line via the
 * `install.start` RPC. Cancel kills the install + removes the partial
 * dir.
 *
 * UX: modal opens in `idle` state with a "Click to install" CTA at the
 * bottom-right. Clicking starts the install; the CTA goes away and the
 * xterm renders npm's output (ANSI colors preserved).
 */

import { Show, createEffect, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";

import { Button } from "@/element/button";
import { ErrorBanner } from "@/app/errors/ErrorBanner";
import { atoms } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import { computeTermThemeFromSettings } from "@/app/view/term/termutil";

import { getCliCatalogEntry } from "../defaults/cli-catalog";
import { getProvider } from "../providers";
// Use the project's customized xterm.css copy (same one term.tsx
// imports) rather than the raw package stylesheet. The package CSS
// loads later in the bundle and would override our project-wide
// terminal theme tweaks.
import "../../term/xterm.css";

interface AgentInstallModalPanelProps {
    agent: ForgeAgent;
    onCancel: () => void;
    /**
     * Fires when the install completed successfully. The boolean tells
     * the caller whether the user clicked "Continue to Launch" (true)
     * or "Close" (false). The picker uses the false case to still flip
     * its cached install state so the ribbon goes away even when the
     * user dismisses the success screen — codex caught this on PR #895.
     */
    onInstalled: (continueToLaunch: boolean) => void;
}

export const AgentInstallModalPanel = (props: AgentInstallModalPanelProps): JSX.Element => {
    const catalog = () => getCliCatalogEntry(props.agent.provider);
    const provider = () => getProvider(props.agent.provider);
    const displayName = () => catalog()?.displayName ?? props.agent.name;

    const [phase, setPhase] = createSignal<"idle" | "installing" | "done" | "failed">("idle");
    // `unknown` — accepts plain strings (legacy) AND the wire-format
    // `AgentMuxError` object the backend now emits for typed errors.
    // `<ErrorBanner>` + `translateError()` handle both shapes.
    const [error, setError] = createSignal<unknown>(null);
    const [sessionId, setSessionId] = createSignal<string | null>(null);
    const [elapsedMs, setElapsedMs] = createSignal(0);
    // When true, install runs with `npm --loglevel=verbose` + progress
    // bar enabled. Off by default — keeps the install log scannable.
    const [verbose, setVerbose] = createSignal(false);

    let unsub: (() => void) | null = null;
    let termRef: HTMLDivElement | undefined;
    let terminal: Terminal | null = null;
    let fitAddon: FitAddon | null = null;
    let resizeObserver: ResizeObserver | null = null;
    let startedAt = 0;
    let tickHandle: ReturnType<typeof setInterval> | null = null;
    // Flipped in onCleanup so a startInstall awaiting the RPC response
    // can cancel the resolved session id even if it landed after unmount.
    let disposed = false;
    // Set when either footer button fires onInstalled, so the unmount
    // path in onCleanup doesn't double-fire. Also lets us detect "user
    // dismissed the success screen via ESC / backdrop" (notifiedDone
    // stays false in those paths) and flip state once on the way out.
    // Codex P2 on PR #895.
    let notifiedDone = false;

    const writeTerm = (line: string, stream: "stdout" | "stderr") => {
        if (!terminal) return;
        if (stream === "stderr") {
            terminal.write(`\x1b[31m${line}\x1b[0m\r\n`);
        } else {
            terminal.write(`${line}\r\n`);
        }
    };

    const startInstall = async () => {
        const prov = provider();
        if (!prov) {
            setError(`unknown provider ${props.agent.provider}`);
            setPhase("failed");
            return;
        }
        // Tear down any prior run (Retry path).
        if (unsub) {
            unsub();
            unsub = null;
        }
        if (tickHandle != null) {
            clearInterval(tickHandle);
            tickHandle = null;
        }
        if (terminal) {
            terminal.clear();
        }
        setPhase("installing");
        setError(null);
        startedAt = Date.now();
        tickHandle = setInterval(() => setElapsedMs(Date.now() - startedAt), 250);
        try {
            const r = await RpcApi.InstallStartCommand(TabRpcClient, {
                providerId: prov.id,
                cliCommand: prov.cliCommand,
                npmPackage: prov.npmPackage,
                pinnedVersion: prov.pinnedVersion,
                verbose: verbose(),
            });
            // If the modal unmounted while the RPC was in flight, cancel
            // the resolved session id rather than subscribing.
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
                        writeTerm(data.line, data.stream === "stderr" ? "stderr" : "stdout");
                    } else if (data.op === "done") {
                        if (data.ok) {
                            // Don't auto-chain — the user clicks
                            // "Continue to Launch" in the footer so
                            // they have a moment to read the install
                            // log and confirm the operation
                            // succeeded.
                            setPhase("done");
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
        // Lazy-create the terminal so it doesn't render before the
        // container has a layout size (FitAddon needs a real rect).
        if (!termRef) return;
        // Resolve the project's monospace font at runtime — xterm.js
        // doesn't parse CSS variables, so passing the literal
        // `var(--termfontfamily, ...)` string would silently fall
        // back to xterm's default (Courier), which renders wider
        // than the rest of the app's terminals.
        const cs = getComputedStyle(termRef);
        const termFont = cs.getPropertyValue("--termfontfamily").trim()
            || `"JetBrains Mono", "Fira Code", "Consolas", monospace`;
        // Bind to the same theme source the regular term pane uses
        // (single source of truth — see SPEC_INSTALL_MODAL_TERM_THEME_BINDING_2026_05_18.md).
        const [initialTheme] = computeTermThemeFromSettings(atoms.fullConfigAtom());
        terminal = new Terminal({
            cursorBlink: false,
            scrollback: 5000,
            fontSize: 12,
            fontFamily: termFont,
            theme: initialTheme,
            convertEol: false,
            scrollOnUserInput: false,
        });
        // Live theme swap — mirrors TermThemeUpdater so settings changes
        // while the modal is open take effect without remount.
        createEffect(() => {
            const [t] = computeTermThemeFromSettings(atoms.fullConfigAtom());
            if (terminal) terminal.options.theme = t;
        });
        fitAddon = new FitAddon();
        terminal.loadAddon(fitAddon);
        terminal.open(termRef);
        const tryFit = () => {
            try {
                fitAddon?.fit();
            } catch {
                /* container still 0×0 — wait for next resize */
            }
        };
        tryFit();
        // Refit on container resize. The modal may animate in from
        // 0×0; without an observer the terminal stays at the default
        // 80×24 indefinitely.
        resizeObserver = new ResizeObserver(() => tryFit());
        resizeObserver.observe(termRef);
        terminal.writeln("\x1b[90m# Click \"Install now\" to begin.\x1b[0m");
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
        if (resizeObserver) {
            resizeObserver.disconnect();
            resizeObserver = null;
        }
        if (terminal) {
            terminal.dispose();
            terminal = null;
        }
        const sid = sessionId();
        if (sid && phase() === "installing") {
            void RpcApi.InstallCancelCommand(TabRpcClient, { sessionId: sid }).catch(() => {
                /* best-effort */
            });
        }
        // ESC, backdrop click, or any other unmount path that bypassed
        // the footer buttons. If the install succeeded, we still owe
        // the picker the state flip so the card's ribbon clears.
        // Codex P2 on PR #895.
        if (phase() === "done" && !notifiedDone) {
            props.onInstalled(false);
        }
    });

    const elapsedLabel = () => {
        const s = Math.floor(elapsedMs() / 1000);
        const mm = Math.floor(s / 60).toString();
        const ss = (s % 60).toString().padStart(2, "0");
        return `${mm}:${ss}`;
    };

    return (
        <>
            <header class="modal-panel-header">
                <h2 class="modal-panel-title">
                    <span class="agent-install-modal-icon" aria-hidden="true">
                        {catalog()?.icon ?? "📦"}
                    </span>
                    Install {displayName()}
                </h2>
                <p class="modal-panel-description">
                    <Show when={phase() === "idle"}>not installed — click below to install</Show>
                    <Show when={phase() === "installing"}>
                        <span class="agent-install-modal-spinner">⏳</span> Installing… {elapsedLabel()}
                    </Show>
                    <Show when={phase() === "done"}>
                        <span class="agent-install-modal-ok">✓</span> Installed
                    </Show>
                    <Show when={phase() === "failed"}>
                        <span class="agent-install-modal-fail">✗</span> Failed
                    </Show>
                </p>
            </header>
            <div class="modal-panel-body agent-install-modal-body">
                <div class="agent-install-modal-term" ref={termRef} />
                <Show when={error()}>
                    <ErrorBanner error={error()} />
                </Show>
            </div>
            <footer class="modal-panel-footer">
                <Show when={phase() === "idle"}>
                    <label class="agent-install-modal-verbose">
                        <input
                            type="checkbox"
                            checked={verbose()}
                            onChange={(e) => setVerbose(e.currentTarget.checked)}
                        />
                        <span>Verbose output</span>
                    </label>
                    <Button onClick={() => props.onCancel()}>Cancel</Button>
                    <Button onClick={() => void startInstall()} className="green solid">
                        Install now
                    </Button>
                </Show>
                <Show when={phase() === "installing"}>
                    <Button onClick={() => void cancel()}>Cancel</Button>
                </Show>
                <Show when={phase() === "failed"}>
                    <Button onClick={() => props.onCancel()}>Close</Button>
                    <Button onClick={() => void startInstall()} className="green solid">
                        Retry
                    </Button>
                </Show>
                <Show when={phase() === "done"}>
                    <Button onClick={() => { notifiedDone = true; props.onInstalled(false); }}>Close</Button>
                    <Button onClick={() => { notifiedDone = true; props.onInstalled(true); }} className="green solid">
                        Continue to Launch
                    </Button>
                </Show>
            </footer>
        </>
    );
};

AgentInstallModalPanel.displayName = "AgentInstallModalPanel";
