// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentInstallModalPanel — modal that runs an agent's install recipe.
 * Opens when the user picks an agent whose CLI isn't already in the
 * per-version cache. Sibling to `AgentLaunchModalPanel`.
 *
 * Phase α (SPEC_AGENT_INSTALL_STAGE_2026_05_17.md §11): single-step
 * recipe (just `npm install <package>`) streamed line-by-line via the
 * `install.start` RPC. Cancel kills the install + removes the partial
 * dir.
 *
 * Two layers (SPEC_UNIVERSAL_INSTALL_DIALOG_2026_09_23.md §4, phase 1):
 * the default view is a short list of plain-language steps derived from
 * npm's output by `NpmStepTracker`; the raw console sits in a collapsed
 * "Details" panel (xterm.js, ANSI colours preserved) that opens by itself
 * on failure at the first error line.
 */

import { Show, createEffect, createResource, createSignal, onCleanup, type JSX } from "solid-js";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";

import { Button } from "@/element/button";
import { InstallSteps } from "@/element/install/InstallSteps";
import { NpmStepTracker, type InstallFailure, type InstallStep, type LineTone } from "@/element/install/npm-steps";
import { ErrorBanner } from "@/app/errors/ErrorBanner";
import { atoms, getSettingsKeyAtom } from "@/app/store/global";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { muxEventSubscribe } from "@/app/store/mps";
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
import type { AgentDefinition } from "@/app/store/rpc-api";

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

// Frontend view cap (spec §4.2). The backend keeps the complete log.
const MAX_LOG_LINES = 20_000;
const TRIM_CHUNK = 1_000;

// ANSI colour per tone. Only real errors and warnings are coloured —
// npm writes all of its verbose chatter to stderr, so painting stderr
// red made a healthy install look like a wall of failures (spec §1.1).
const TONE_SGR: Record<LineTone, string> = {
    normal: "",
    command: "\x1b[90m",
    warning: "\x1b[33m",
    error: "\x1b[31m",
};

// Header subtitle per phase: what the install needs before it runs,
// then where it stands.
const DESCRIPTION = {
    idle: "Needs an internet connection.",
    installing: "Needs an internet connection.",
    done: "Ready to launch.",
    failed: "The install didn't finish.",
} as const;

// The user's open/closed choice for Details, remembered for the session
// (spec §4.2). Collapsed by default.
let detailsOpenPref = false;

interface LogLine {
    text: string;
    tone: LineTone;
}

/** An `install_chunk` event payload: a log line, or the final `done`. */
interface InstallChunk {
    line?: string;
    stream?: "stdout" | "stderr";
    op?: "done";
    ok?: boolean;
    error?: unknown;
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
    const [failure, setFailure] = createSignal<InstallFailure | null>(null);
    const [sessionId, setSessionId] = createSignal<string | null>(null);
    const [elapsedMs, setElapsedMs] = createSignal(0);
    const [detailsOpen, setDetailsOpen] = createSignal(detailsOpenPref);

    // Layer 1. Before the first run the plan is shown with every step
    // pending, so the user sees what will happen before clicking.
    let tracker: NpmStepTracker | null = null;
    const [runSteps, setRunSteps] = createSignal<InstallStep[] | null>(null);
    const steps = (): InstallStep[] => runSteps() ?? new NpmStepTracker(displayName()).snapshot();
    const syncSteps = () => {
        if (tracker) setRunSteps(tracker.snapshot());
    };

    // Layer 2's source of truth. The terminal is created lazily the
    // first time Details opens and is then fed from here, so a
    // collapsed Details costs no xterm rendering and a terminal opened
    // late still shows everything.
    let log: LogLine[] = [];
    let trimmedLines = 0;
    let fedLines = 0;

    let unsub: (() => void) | null = null;
    let termRef: HTMLDivElement | undefined;
    let detailsRef: HTMLDetailsElement | undefined;
    let terminal: Terminal | null = null;
    let fitAddon: FitAddon | null = null;
    let terminalFitted = false;
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

    const pumpTerminal = () => {
        if (!terminal || !terminalFitted || fedLines >= log.length) return;
        let chunk = "";
        for (let i = fedLines; i < log.length; i++) {
            const { text, tone } = log[i];
            const sgr = TONE_SGR[tone];
            chunk += sgr ? `${sgr}${text}\x1b[0m\r\n` : `${text}\r\n`;
        }
        fedLines = log.length;
        terminal.write(chunk);
    };

    const appendLog = (text: string, tone: LineTone) => {
        log.push({ text, tone });
        if (log.length > MAX_LOG_LINES) {
            log = log.slice(TRIM_CHUNK);
            trimmedLines += TRIM_CHUNK;
            fedLines = Math.max(0, fedLines - TRIM_CHUNK);
        }
        pumpTerminal();
    };

    // Scroll Details so the first error line sits near the top. Waits
    // for xterm to finish parsing pending writes, then searches the
    // buffer for the line's text — row indices change whenever the
    // terminal reflows, so a remembered row would drift. A long line
    // wraps across several rows in a narrow pane, so each logical line
    // is rebuilt from its continuation rows before matching (codex P2
    // on #3661).
    const scrollToFirstError = () => {
        const index = failure()?.firstErrorLine;
        if (!terminal || index == null) return;
        const target = log[index - trimmedLines]?.text;
        if (!target) return;
        const needle = target.slice(0, 60);
        terminal.write("", () => {
            const buf = terminal?.buffer?.active;
            if (!buf) return;
            for (let i = 0; i < buf.length; i++) {
                const row = buf.getLine(i);
                if (!row || row.isWrapped) continue;
                const rows = [row];
                for (let next = buf.getLine(i + 1); next?.isWrapped; next = buf.getLine(i + rows.length)) {
                    rows.push(next);
                }
                // Every row but the last is full width, so its trailing
                // spaces are content; only the last row is trimmed.
                const text = rows.map((r, k) => r.translateToString(k === rows.length - 1)).join("");
                if (text.startsWith(needle)) {
                    terminal?.scrollToLine(Math.max(0, i - 2));
                    return;
                }
            }
        });
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
        log = [];
        trimmedLines = 0;
        fedLines = 0;
        terminal?.clear();
        tracker = new NpmStepTracker(getCliCatalogEntry(prov.id)?.displayName ?? displayName());
        tracker.start();
        syncSteps();
        setFailure(null);
        setPhase("installing");
        setError(null);
        startedAt = Date.now();
        tickHandle = setInterval(() => {
            setElapsedMs(Date.now() - startedAt);
            tracker?.tick(Date.now());
            syncSteps();
        }, 250);
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
            unsub = muxEventSubscribe({
                eventType: "install_chunk",
                scope: `install:${r.sessionId}`,
                handler: (event: { data?: InstallChunk }) => {
                    const data = event?.data;
                    if (!data || typeof data !== "object") return;
                    if (typeof data.line === "string") {
                        const tone = tracker?.line(data.line, Date.now()) ?? "normal";
                        appendLog(data.line, tone);
                        syncSteps();
                    } else if (data.op === "done") {
                        if (tickHandle != null) {
                            clearInterval(tickHandle);
                            tickHandle = null;
                        }
                        if (data.ok) {
                            tracker?.succeed();
                            syncSteps();
                            // Don't auto-chain — the user clicks
                            // "Continue to Launch" in the footer.
                            setPhase("done");
                        } else {
                            fail(data.error ?? "install failed");
                        }
                    }
                },
            });
        } catch (e) {
            fail((e as Error)?.message ?? String(e));
        }
    };

    const fail = (err: unknown) => {
        if (tickHandle != null) {
            clearInterval(tickHandle);
            tickHandle = null;
        }
        setError(err);
        if (tracker) {
            setFailure(tracker.fail(err));
            syncSteps();
        }
        setPhase("failed");
        // Spec §7: Details opens by itself on failure, at the first error.
        // A failure doesn't change the remembered preference.
        setDetailsOpen(true);
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

    const tryFit = () => {
        try {
            fitAddon?.fit();
            if (terminal && terminal.cols > 2) {
                terminalFitted = true;
                pumpTerminal();
            }
        } catch {
            /* container still 0×0 — wait for next resize */
        }
    };

    const ensureTerminal = () => {
        if (terminal || !termRef) return;
        // Resolve the project's monospace font at runtime — xterm.js
        // doesn't parse CSS variables, so passing a literal `var(...)`
        // string would silently fall back to xterm's default (Courier),
        // which renders wider than the rest of the app's terminals.
        const cs = getComputedStyle(termRef);
        // Reads --font-mono, the canonical family token (PR #3252).
        const termFont = cs.getPropertyValue("--font-mono").trim()
            || `"Hack", Consolas, Menlo, monospace`;
        // Bind to the same theme source the regular term pane uses
        // (single source of truth — see SPEC_INSTALL_MODAL_TERM_THEME_BINDING_2026_05_18.md).
        const [initialTheme] = computeTermThemeFromSettings(atoms.fullConfigAtom());
        // Layer D1 of MODAL_COMPACT_VARIANT_ARCHITECTURE_2026_05_26 §7:
        // construct at the smallest viable size (2×2) instead of the
        // xterm.js default of 80×24, so the first paint can't widen the
        // panel before FitAddon sizes it. Nothing is written until the
        // first successful fit (`terminalFitted`), so no line is ever
        // wrapped into 2-column rows and pushed out of scrollback.
        const term = new Terminal({
            cursorBlink: false,
            scrollback: 5000,
            fontSize: 12,
            fontFamily: termFont,
            theme: initialTheme,
            convertEol: false,
            scrollOnUserInput: false,
            disableStdin: true,
            cols: 2,
            rows: 2,
        });
        terminal = term;
        // Clipboard wiring — phase α of SPEC_UNIFIED_CLIPBOARD_2026_05_18.md.
        // Mirrors the regular term pane's copy-on-select and Ctrl+Shift+C.
        const copyOnSelect = getSettingsKeyAtom("term:copyonselect");
        // Debounce matches termwrap.ts:205 — fires once per drag burst.
        term.onSelectionChange(() => {
            if (!copyOnSelect()) return;
            if (selectionDebounce != null) clearTimeout(selectionDebounce);
            selectionDebounce = setTimeout(() => {
                const sel = terminal?.getSelection() ?? "";
                if (sel.length > 0) {
                    clipboardWriteText(sel).catch((e) => console.log("clipboard write failed", e));
                }
            }, 50);
        });
        term.attachCustomKeyEventHandler((ev) => {
            // Ctrl+Shift+C → manual copy. Return false stops xterm from
            // also routing the keystroke as input.
            if (ev.type === "keydown" && ev.ctrlKey && ev.shiftKey && ev.key === "C") {
                const sel = terminal?.getSelection() ?? "";
                if (sel.length > 0) {
                    clipboardWriteText(sel).catch((e) => console.log("clipboard write failed", e));
                }
                return false;
            }
            return true;
        });

        fitAddon = new FitAddon();
        term.loadAddon(fitAddon);
        term.open(termRef);
        // Force-load the term font BEFORE refitting so cell-width
        // measurement uses real glyph metrics, not fallback (Courier).
        // Same race as termwrap.ts — see
        // docs/archive/terminal-jumbled-startup-investigation.md "Follow-up".
        const FIT_FONT_TIMEOUT_MS = 1000;
        const fontSpec = (variant: string) => `${variant}12px ${termFont}`;
        void (async () => {
            try {
                await Promise.race([
                    Promise.all([
                        document.fonts?.load(fontSpec("")) ?? Promise.resolve(),
                        document.fonts?.load(fontSpec("bold ")) ?? Promise.resolve(),
                        document.fonts?.load(fontSpec("italic ")) ?? Promise.resolve(),
                    ]),
                    new Promise<void>((resolve) => setTimeout(resolve, FIT_FONT_TIMEOUT_MS)),
                ]);
            } catch { /* font API unavailable — fall through */ }
            if (!disposed) tryFit();
        })();
        tryFit(); // best-effort initial fit (often fallback metrics on cold cache)
        resizeObserver = new ResizeObserver(() => tryFit());
        resizeObserver.observe(termRef);
    };

    // Live theme swap — mirrors TermThemeUpdater so settings changes
    // while the modal is open take effect without remount.
    createEffect(() => {
        const [t] = computeTermThemeFromSettings(atoms.fullConfigAtom());
        if (terminal) terminal.options.theme = t;
    });

    // Opening Details creates or refits the terminal. A closed <details>
    // doesn't lay out its children, so FitAddon can't size until it's
    // open. Runs after the `open` attribute is applied to the element.
    // Tracks `phase` too: a failure while Details is already open must
    // still scroll to the first error, and re-setting an already-true
    // `detailsOpen` would not re-run this (codex P2 on #3661).
    createEffect(() => {
        const failed = phase() === "failed";
        if (!detailsOpen()) return;
        queueMicrotask(() => {
            if (disposed || !detailsRef?.open) return;
            ensureTerminal();
            tryFit();
            if (failed) scrollToFirstError();
        });
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

    const copyAll = () => {
        const lines = log.map((l) => l.text);
        if (trimmedLines > 0) lines.unshift(`[${trimmedLines} earlier lines trimmed]`);
        const all = lines.join("\n");
        if (all.length === 0) return;
        void clipboardWriteText(all).catch((err) => console.log("clipboard write failed", err));
    };

    // Typed backend errors (e.g. disk full while creating the install
    // directory) carry a friendlier message than the category line.
    const typedError = () => {
        const e = error();
        return e != null && typeof e !== "string" ? e : null;
    };

    return (
        <div class="agent-install-modal">
            <header class="modal-panel-header">
                <h2 class="modal-panel-title">
                    <span class="agent-install-modal-icon" aria-hidden="true">
                        {catalog()?.icon ?? "📦"}
                    </span>
                    {phase() === "done" ? `${displayName()} is installed` : `Install ${displayName()}`}
                    <Show when={version()}>
                        <span class="agent-install-modal-version">
                            {version() === "latest" ? "latest" : `v${version()}`}
                        </span>
                    </Show>
                </h2>
                <p class="modal-panel-description">{DESCRIPTION[phase()]}</p>
            </header>
            <div class="modal-panel-body agent-install-modal-body">
                <InstallSteps steps={steps()} />
                <Show when={phase() === "failed" && !failure() && typeof error() === "string"}>
                    {/* Failed before a step plan existed (e.g. unknown provider). */}
                    <ErrorBanner error={error()} />
                </Show>
                <Show when={typedError()}>
                    <ErrorBanner error={typedError()} />
                </Show>
                <details
                    class="agent-install-modal-details"
                    open={detailsOpen()}
                    ref={detailsRef}
                    onToggle={(e) => {
                        const open = e.currentTarget.open;
                        if (open === detailsOpen()) return;
                        setDetailsOpen(open);
                        detailsOpenPref = open;
                    }}
                >
                    <summary>Details</summary>
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
                                        enabled: log.length > 0,
                                        click: copyAll,
                                    },
                                ],
                                e,
                            );
                        }}
                    />
                </details>
            </div>
            <footer class="modal-panel-footer">
                <Show when={phase() !== "idle"}>
                    <span class="agent-install-modal-elapsed" aria-label="Elapsed time">
                        {elapsedLabel()}
                    </span>
                </Show>
                <Show when={phase() === "idle"}>
                    <Button onClick={() => props.onCancel()} data-modal-dismiss>Cancel</Button>
                    <Button onClick={() => void startInstall()} className="green solid" data-modal-initial-focus>
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
        </div>
    );
};

AgentInstallModalPanel.displayName = "AgentInstallModalPanel";
