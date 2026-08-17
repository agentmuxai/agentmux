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

import { Show, createEffect, createResource, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";

import { Button } from "@/element/button";
import { ErrorBanner } from "@/app/errors/ErrorBanner";
import { atoms, getSettingsKeyAtom } from "@/app/store/global";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import { computeTermThemeFromSettings } from "@/app/view/term/termutil";
import { writeText as clipboardWriteText } from "@/util/clipboard";

import { getCliCatalogEntry } from "../defaults/cli-catalog";
import { getProvider } from "../providers";
import { resolveEffectiveLaunchProvider } from "../agent-launch-env";
// Use the project's customized xterm.css copy (same one term.tsx
// imports) rather than the raw package stylesheet. The package CSS
// loads later in the bundle and would override our project-wide
// terminal theme tweaks.
import "../../term/xterm.css";

interface AgentInstallModalPanelProps {
    agent: AgentDefinition;
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
    // Resolve through the agent's bound bundle rather than the possibly-
    // drifted `agent.provider` column directly — #2594, same "gate vs.
    // actual launch can disagree" risk class #2592/#2596/#2607/#2609
    // fixed. This modal determines which CLI package literally gets
    // installed (`startInstall` below); disagreeing with what
    // AgentPicker's checkInstalled (already fixed) decided needed
    // installing would install the wrong provider's CLI.
    //
    // Used only for the cosmetic header (icon/displayName/version) —
    // `startInstall` re-resolves directly rather than reading this
    // resource, so a click that races the resource's own in-flight
    // fetch still installs the correct provider (see its own comment).
    // Falls back to `props.agent.provider` while loading/on failure,
    // same as `resolveEffectiveLaunchProvider` itself.
    const [resolvedProviderId] = createResource(() => props.agent, resolveEffectiveLaunchProvider);
    const displayProviderId = () => resolvedProviderId() ?? props.agent.provider;
    const catalog = () => getCliCatalogEntry(displayProviderId());
    const provider = () => getProvider(displayProviderId());
    const displayName = () => catalog()?.displayName ?? props.agent.name;
    const version = () => provider()?.pinnedVersion;

    const [phase, setPhase] = createSignal<"idle" | "installing" | "done" | "failed">("idle");
    // `unknown` — accepts plain strings (legacy) AND the wire-format
    // `AgentMuxError` object the backend now emits for typed errors.
    // `<ErrorBanner>` + `translateError()` handle both shapes.
    const [error, setError] = createSignal<unknown>(null);
    const [sessionId, setSessionId] = createSignal<string | null>(null);
    const [elapsedMs, setElapsedMs] = createSignal(0);

    let unsub: (() => void) | null = null;
    let termRef: HTMLDivElement | undefined;
    let terminal: Terminal | null = null;
    let fitAddon: FitAddon | null = null;
    let resizeObserver: ResizeObserver | null = null;
    let startedAt = 0;
    let tickHandle: ReturnType<typeof setInterval> | null = null;
    // Hoisted so onCleanup can cancel the pending copy-on-select
    // timer when the modal unmounts (reagent P2 on PR #899 v2).
    let selectionDebounce: ReturnType<typeof setTimeout> | null = null;
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
        // Re-resolve directly rather than reading the `provider()`
        // memo above — that memo backs the resource's current
        // (possibly still-loading, or subsequently-stale if the
        // component has been open a while) snapshot, whereas a fresh
        // resolve here guarantees whatever actually gets installed
        // matches the agent's bundle at the moment the user clicked,
        // not whatever the header happened to be showing.
        // resolveEffectiveLaunchProvider is a cheap, idempotent single
        // RPC round-trip — no reason to trust a possibly-stale cache
        // for the one call that determines what gets installed.
        const resolvedId = await resolveEffectiveLaunchProvider(props.agent);
        const prov = getProvider(resolvedId);
        if (!prov) {
            setError(`unknown provider ${resolvedId}`);
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
        // Layer D1 of MODAL_COMPACT_VARIANT_ARCHITECTURE_2026_05_26 §7:
        // construct at the smallest viable size (2×2) instead of the
        // xterm.js default of 80×24. In a narrow agent pane, default
        // cols=80 paints a ~600px-wide canvas BEFORE FitAddon's first
        // ResizeObserver tick can fire — the modal-panel locks in to
        // that width, `.modal-panel { overflow: auto }` reserves a
        // horizontal scrollbar, and the user sees an unshrunk modal
        // even though the parent CSS has `min-width: 0`. Starting at
        // 2×2 means the initial paint is tiny; the synchronous
        // `fitAddon.fit()` after `terminal.open(termRef)` then sizes
        // to the actual container width on the same frame.
        terminal = new Terminal({
            cursorBlink: false,
            scrollback: 5000,
            fontSize: 12,
            fontFamily: termFont,
            theme: initialTheme,
            convertEol: false,
            scrollOnUserInput: false,
            cols: 2,
            rows: 2,
        });
        // Live theme swap — mirrors TermThemeUpdater so settings changes
        // while the modal is open take effect without remount.
        createEffect(() => {
            const [t] = computeTermThemeFromSettings(atoms.fullConfigAtom());
            if (terminal) terminal.options.theme = t;
        });
        // Clipboard wiring — phase α of SPEC_UNIFIED_CLIPBOARD_2026_05_18.md.
        // Mirrors the regular term pane's three paths (copy-on-select,
        // Ctrl+Shift+C, context menu Copy) so users can pull npm output
        // out of the install log.
        const copyOnSelect = getSettingsKeyAtom("term:copyonselect");
        // Debounce matches termwrap.ts:205 — fires once per drag burst
        // instead of once per tick. `selectionDebounce` is hoisted to
        // component scope so onCleanup can cancel it.
        terminal.onSelectionChange(() => {
            if (!copyOnSelect()) return;
            if (selectionDebounce != null) clearTimeout(selectionDebounce);
            selectionDebounce = setTimeout(() => {
                const sel = terminal?.getSelection() ?? "";
                if (sel.length > 0) {
                    clipboardWriteText(sel).catch((e) =>
                        console.log("clipboard write failed", e),
                    );
                }
            }, 50);
        });
        terminal.attachCustomKeyEventHandler((ev) => {
            // Ctrl+Shift+C → manual copy. Return false stops xterm from
            // also routing the keystroke as input.
            if (ev.type === "keydown" && ev.ctrlKey && ev.shiftKey && ev.key === "C") {
                const sel = terminal?.getSelection() ?? "";
                if (sel.length > 0) {
                    clipboardWriteText(sel).catch((e) =>
                        console.log("clipboard write failed", e),
                    );
                }
                return false;
            }
            return true;
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
        // Force-load the term font BEFORE the first fit so cell-width
        // measurement uses real glyph metrics, not fallback (Courier).
        // Same race as termwrap.ts — see
        // docs/archive/terminal-jumbled-startup-investigation.md "Follow-up".
        // fonts.load() actively requests the face and resolves when ready;
        // fonts.ready alone is vacuous because the WOFF/WOFF2 isn't
        // requested until something measures a glyph.
        const FIT_FONT_TIMEOUT_MS = 1000;
        const fontSpec = (variant: string) => `${variant}12px ${termFont}`;
        const fontsReady = (async () => {
            try {
                await Promise.race([
                    Promise.all([
                        document.fonts?.load(fontSpec("")) ?? Promise.resolve(),
                        document.fonts?.load(fontSpec("bold ")) ?? Promise.resolve(),
                        document.fonts?.load(fontSpec("italic ")) ?? Promise.resolve(),
                    ]),
                    new Promise<void>((resolve) =>
                        setTimeout(resolve, FIT_FONT_TIMEOUT_MS),
                    ),
                ]);
            } catch (_) { /* font API unavailable — fall through */ }
        })();
        void fontsReady.then(() => {
            if (disposed) return;
            tryFit();
        });
        tryFit(); // best-effort initial fit (often fallback metrics on cold cache)
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
        if (selectionDebounce != null) {
            clearTimeout(selectionDebounce);
            selectionDebounce = null;
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
                    <Show when={version()}>
                        <span class="agent-install-modal-version">
                            {version() === "latest" ? "latest" : `v${version()}`}
                        </span>
                    </Show>
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
                <div
                    class="agent-install-modal-term"
                    ref={termRef}
                    onContextMenu={(e) => {
                        // Right-click → Copy (selection) / Copy All. Mirrors
                        // the regular term pane's menu. Phase α of
                        // SPEC_UNIFIED_CLIPBOARD_2026_05_18.md.
                        // preventDefault stops Chromium's native right-click
                        // menu from firing alongside our custom one
                        // (reagent P1 + codex P2 on PR #899).
                        e.preventDefault();
                        const sel = terminal?.getSelection() ?? "";
                        // Lazy-walk the scrollback inside the click
                        // handler so we don't pay for it on every
                        // right-click + dismiss (reagent P2). For
                        // long install logs this is non-trivial work.
                        const collectAll = (): string => {
                            // Reassemble logical lines from xterm's visual
                            // rows. `isWrapped` on row N+1 means row N's
                            // logical line continues onto N+1, so emit a
                            // newline only when the next row starts a fresh
                            // logical line. Codex P2 on PR #899 v3 — naive
                            // join inserted artificial breaks into long
                            // URLs / stack traces / npm command echoes.
                            const buf = terminal?.buffer?.active;
                            if (!buf) return "";
                            let out = "";
                            for (let i = 0; i < buf.length; i++) {
                                const line = buf.getLine(i);
                                if (!line) continue;
                                out += line.translateToString(true);
                                const next = buf.getLine(i + 1);
                                if (!next?.isWrapped) out += "\n";
                            }
                            return out;
                        };
                        ContextMenuModel.showContextMenu(
                            [
                                {
                                    label: "Copy",
                                    enabled: sel.length > 0,
                                    click: () => void clipboardWriteText(sel).catch((err) =>
                                        console.log("clipboard write failed", err)),
                                },
                                {
                                    label: "Copy All",
                                    enabled: !!terminal?.buffer?.active?.length,
                                    click: () => {
                                        const all = collectAll();
                                        if (all.length === 0) return;
                                        void clipboardWriteText(all).catch((err) =>
                                            console.log("clipboard write failed", err));
                                    },
                                },
                            ],
                            e,
                        );
                    }}
                />
                <Show when={error()}>
                    <ErrorBanner error={error()} />
                </Show>
            </div>
            <footer class="modal-panel-footer">
                <Show when={phase() === "idle"}>
                    <Button onClick={() => props.onCancel()} data-modal-dismiss>Cancel</Button>
                    <Button onClick={() => void startInstall()} className="green solid">
                        Install now
                    </Button>
                </Show>
                <Show when={phase() === "installing"}>
                    <Button onClick={() => void cancel()} data-modal-dismiss>Cancel</Button>
                </Show>
                <Show when={phase() === "failed"}>
                    <Button onClick={() => props.onCancel()} data-modal-dismiss>Close</Button>
                    <Button onClick={() => void startInstall()} className="green solid">
                        Retry
                    </Button>
                </Show>
                <Show when={phase() === "done"}>
                    <Button onClick={() => { notifiedDone = true; props.onInstalled(false); }} data-modal-dismiss>Close</Button>
                    <Button onClick={() => { notifiedDone = true; props.onInstalled(true); }} className="green solid">
                        Continue to Launch
                    </Button>
                </Show>
            </footer>
        </>
    );
};

AgentInstallModalPanel.displayName = "AgentInstallModalPanel";
