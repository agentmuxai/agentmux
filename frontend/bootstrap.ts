// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Application bootstrap — entry point loaded by index.html.
// Initializes logging, detects the host runtime, sets up the API bridge,
// then launches the main application (app-init.ts).

import { initLogPipe } from "./log/log-pipe";
import { initErrorForwarder } from "./log/error-forwarder";
import { setupCefApi } from "./cef-init";
import { initApp } from "./app-init";
import { tryAutoRecover, clearStartupReloadCount } from "./app/init/error-display";
import { benchMark } from "@/util/startup-bench";
import { initPerf } from "@/perf";
import { invokeCommand } from "@/app/platform/ipc";

// Pipe all console.log/warn/error to the Rust host log file.
// Must run before any other code so early messages are captured.
initLogPipe();

// ── First-paint signal (Linux startup white-flash fix) ──────────────────────
// docs/specs/REPORT_NEW_WINDOW_STARTUP_COLOR_FLASH_2026_07_14.md.
//
// Tell the host the moment the browser has actually composited a frame — not
// "main-frame load complete" (CEF's `on_load_end`, which can fire before
// anything has visually painted and is what the host used to gate the native
// window's show() on). Double rAF is the standard proxy for "a frame was
// presented": the first callback runs before this frame is drawn, the second
// only after it has been.
//
// Fired directly via invokeCommand() rather than getApi() — getApi() isn't
// installed until setupCefApi()'s full IPC batch resolves, which can take
// seconds on a slow start (see the spec's profiling numbers) and would be far
// too late to gate the window's first show(). invokeCommand() only needs
// `window.__AGENTMUX_IPC_PORT__`/`__AGENTMUX_IPC_TOKEN__`, which on a normal
// (non-reload) launch are already present from the boot URL's query params —
// no wait required. Harmless no-op outside CEF (invokeCommand rejects; caught
// and ignored) and on platforms that don't gate on this signal (Windows/macOS
// currently just log it).
requestAnimationFrame(() => {
    requestAnimationFrame(() => {
        benchMark("frontend-painted");
        const label = new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
        invokeCommand("report_first_paint", { label }).catch(() => {});
    });
});

// Capture uncaught errors + unhandled promise rejections and forward
// them via the same fe_log_structured IPC channel as the console pipe.
// SolidJS reconciler DOM exceptions (e.g. replaceChild NotFoundError)
// surface only as window "error" events and would otherwise leave no
// trace in the host log. Retro 2026-05-23 (agent-pane cascade →
// replaceChild quick-win).
initErrorForwarder();

// Phase 0 perf instrumentation: Long Tasks observer + INP/event
// observer. Must run before any user-interactive code so we never
// miss a "first interaction" sample. See
// docs/specs/SPEC_PERFORMANCE_INSTRUMENTATION_AND_OPTIMIZATION.md.
initPerf();

// ── GPU context loss recovery ───────────────────────────────────────────────
// Recover from GPU context loss (driver reset, DXGI device removal, display
// power state change). Reload the page to re-establish the rendering surface.
// Reload-loop protection: stop after 3 attempts to avoid infinite reloads.
// Counter resets when the page loads successfully (60s without context loss).

const CONTEXT_LOSS_MAX_RELOADS = 3;
const CONTEXT_LOSS_STORAGE_KEY = "webgl-context-loss-reloads";
let contextLostReloading = false;

function getContextLossReloadCount(): number {
    try {
        return parseInt(sessionStorage.getItem(CONTEXT_LOSS_STORAGE_KEY) ?? "0", 10) || 0;
    } catch {
        return 0;
    }
}

function setContextLossReloadCount(n: number) {
    try {
        sessionStorage.setItem(CONTEXT_LOSS_STORAGE_KEY, String(n));
    } catch {
        // sessionStorage unavailable
    }
}

setTimeout(() => setContextLossReloadCount(0), 60_000);

document.addEventListener("webglcontextlost", (event) => {
    event.preventDefault();
    if (contextLostReloading) return;
    const reloadCount = getContextLossReloadCount();
    if (reloadCount >= CONTEXT_LOSS_MAX_RELOADS) {
        console.error(`[recovery] WebGL context lost — suppressing reload (already reloaded ${reloadCount}x, possible driver issue)`);
        return;
    }
    contextLostReloading = true;
    setContextLossReloadCount(reloadCount + 1);
    console.error(`[recovery] WebGL context lost — reloading page in 1s (attempt ${reloadCount + 1}/${CONTEXT_LOSS_MAX_RELOADS})`);
    setTimeout(() => window.location.reload(), 1000);
}, true);

// ── Keyboard reload (host-window recovery) ───────────────────────────────────
// AgentMux's app window is CEF, not a browser, so Ctrl+R / F5 aren't wired to a
// page reload (the host's only reload path is for browser *panes*). Register a
// capture-phase handler BEFORE the bridge handshake so it works even when the
// UI is wedged on a startup failure — the exact state where users reach for it.
//
// SCOPED TO STARTUP ONLY: this handler is removed the moment the app finishes
// loading (see the success path in bootstrap()). Leaving it active for the
// whole session would hijack pane-level Ctrl+R — terminal reverse-i-search
// (bash/zsh/readline), editor shortcuts — and reload the whole app instead.
// [reagent #1424 P1]
const startupReloadKeyHandler = (e: KeyboardEvent) => {
    const isReload =
        e.key === "F5" || ((e.ctrlKey || e.metaKey) && (e.key === "r" || e.key === "R"));
    if (isReload) {
        e.preventDefault();
        window.location.reload();
    }
};
window.addEventListener("keydown", startupReloadKeyHandler, true);

// ── Static CSS imports ──────────────────────────────────────────────────────
import "overlayscrollbars/overlayscrollbars.css";
import "./app/app.scss";
import "./tailwindsetup.css";

// ── Logging ─────────────────────────────────────────────────────────────────
const log = (level: string, ...args: any[]) => {
    const timestamp = new Date().toISOString();
    console.log(`[${timestamp}] [${level}]`, ...args);
    try {
        if (window.api?.sendLog) {
            window.api.sendLog(`[${level}] ${args.join(' ')}`);
        }
    } catch {
        // Ignore if backend not ready
    }
};

window.debugLog = log;

// ── Bootstrap ───────────────────────────────────────────────────────────────
async function bootstrap() {
    try {
        benchMark("bootstrap-start");
        log("INFO", "=== AgentMux Bootstrap Starting ===");
        log("INFO", "User Agent:", navigator.userAgent);
        log("INFO", "Location:", window.location.href);

        if (import.meta.env.DEV) {
            console.log("%c[DEV MODE] Loading from Vite dev server — HMR active", "color: lime; font-size: 14px; font-weight: bold");
        } else {
            console.warn("%c[PRODUCTION BUILD] Loading from dist/frontend — source changes will NOT hot-reload!", "color: red; font-size: 14px; font-weight: bold");
        }

        // Initialize the host API bridge
        log("INFO", "Initializing API...");
        benchMark("setupCefApi-start");
        await setupCefApi();
        benchMark("setupCefApi-done");
        log("INFO", "API initialized, window.api available:", !!window.api);

        // Floating pane: when the host opens a window via
        // `open_floating_pane_window` (SPEC_FLOATING_PANE_TEAROFF), it
        // appends `?floatingPaneId=<id>&workspaceId=<id>` to the URL.
        // Phase 1 (#810) short-circuited bootstrap to render a placeholder
        // shell. Phase 2 (#1077) lets the standard `initApp` →
        // `initHostNewWindow` path handle it: that path already picks up
        // `?workspaceId=` to attach to the existing workspace, and the
        // `App` component reads `?floatingPaneId=` to render a chromeless
        // single-pane layout (no tab bar / widgets / status bar). One
        // codepath, no synthesized renderers.

        // Launch the main application
        log("INFO", "Starting main application (app-init)...");
        benchMark("initApp-start");
        try {
            await initApp();
            log("INFO", "✅ Main application loaded successfully");
            // Successful startup — reset the auto-reload budget so a later,
            // unrelated failure begins with a full set of retries, and drop the
            // startup reload keybinding so pane-level Ctrl+R (terminal
            // reverse-i-search, editor) works normally. [reagent #1424 P1]
            clearStartupReloadCount();
            window.removeEventListener("keydown", startupReloadKeyHandler, true);
        } catch (initError) {
            log("ERROR", "Failed in initApp:", initError);
            log("ERROR", "Init error name:", (initError as Error)?.name);
            log("ERROR", "Init error message:", (initError as Error)?.message);
            log("ERROR", "Init error stack:", (initError as Error)?.stack);
            throw initError;
        }

    } catch (error) {
        log("ERROR", "❌ Bootstrap failed:", error);
        log("ERROR", "Stack:", (error as Error).stack);

        // Self-heal: a bridge-init failure is almost always transient (a Vite
        // full-reload mid-change in dev, a slow backend spawn, or a stale
        // pooled window). tryAutoRecover does a bounded auto-reload; only when
        // the budget is exhausted does it render the recovery card (with a
        // manual Reload). Either way the user never gets the old dead screen.
        const detail = `${String(error)}\n\n${(error as Error)?.stack ?? ""}`.trim();
        const reloading = tryAutoRecover(detail);
        if (!reloading) {
            // Recovery card is shown; re-throw so the structured error
            // forwarder still ships the failure to the host log.
            throw error;
        }
        // Page is about to reload — swallow so no UI flashes before navigation.
    }
}

// Uncaught error / unhandled promise rejection forwarding is handled
// by initErrorForwarder() at the top of this file. That path captures
// stack / name / source and ships via the same fe_log_structured IPC
// channel as the console pipe, so DOM exceptions from the SolidJS
// reconciler land in the host log.

bootstrap();
