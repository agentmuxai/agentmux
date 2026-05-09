// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Application bootstrap — entry point loaded by index.html.
// Initializes logging, detects the host runtime, sets up the API bridge,
// then launches the main application (app-init.ts).

import { initLogPipe } from "./log/log-pipe";
import { setupCefApi } from "./cef-init";
import { initApp } from "./app-init";
import { benchMark } from "@/util/startup-bench";
import { initPerf } from "@/perf";

// Pipe all console.log/warn/error to the Rust host log file.
// Must run before any other code so early messages are captured.
initLogPipe();

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

        // Launch the main application
        log("INFO", "Starting main application (app-init)...");
        benchMark("initApp-start");
        try {
            await initApp();
            log("INFO", "✅ Main application loaded successfully");
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

        document.body.innerHTML = "";
        const errorDiv = document.createElement("div");
        errorDiv.style.cssText = "padding: 20px; font-family: monospace; color: red;";

        const title = document.createElement("h1");
        title.textContent = "AgentMux Failed to Start";
        errorDiv.appendChild(title);

        const errorPre = document.createElement("pre");
        errorPre.textContent = String(error);
        errorDiv.appendChild(errorPre);

        const stackPre = document.createElement("pre");
        stackPre.textContent = (error as Error).stack || "";
        errorDiv.appendChild(stackPre);

        const helpText = document.createElement("p");
        helpText.textContent = "Check the browser console (F12) for more details.";
        errorDiv.appendChild(helpText);

        document.body.appendChild(errorDiv);
        throw error;
    }
}

// Capture unhandled errors to backend log
window.addEventListener("error", (event) => {
    log("UNCAUGHT-ERROR", event.message, "at", event.filename, "line", String(event.lineno));
});
window.addEventListener("unhandledrejection", (event) => {
    log("UNHANDLED-REJECTION", event.reason?.message ?? String(event.reason));
});

bootstrap();
