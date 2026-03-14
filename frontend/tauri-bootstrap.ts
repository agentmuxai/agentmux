// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tauri bootstrap with verbose logging.
// This is the true entry point for Tauri builds.

import { setupTauriApi } from "./tauri-init";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { readTextFile, exists } from "@tauri-apps/plugin-fs";

// Static CSS imports so Vite includes them in the HTML <link> tags.
// wave.ts is dynamically imported (for error handling), but its CSS must
// be loaded eagerly — Tauri's webview doesn't process Vite's dynamic CSS injection.
import "overlayscrollbars/overlayscrollbars.css";
import "./app/app.scss";
import "./tailwindsetup.css";

// Deep verbose logging
const log = (level: string, ...args: any[]) => {
    const timestamp = new Date().toISOString();
    console.log(`[${timestamp}] [${level}]`, ...args);

    // Also log to backend if available
    try {
        if ((window as any).api?.sendLog) {
            (window as any).api.sendLog(`[${level}] ${args.join(' ')}`);
        }
    } catch (e) {
        // Ignore if backend not ready
    }
};

(window as any).debugLog = log;

/**
 * Check for backend startup errors during initialization.
 * The backend writes errors to a startup-error.txt file if it fails to start.
 * We poll for this file during bootstrap and show an error dialog if found.
 */
async function checkBackendStartupError(): Promise<boolean> {
    try {
        const dataDir = await invoke<string>("get_data_dir");
        // Use platform-agnostic path joining
        const errorFilePath = dataDir.endsWith("/") || dataDir.endsWith("\\")
            ? `${dataDir}startup-error.txt`
            : `${dataDir}/startup-error.txt`;

        // Check if error file exists
        const fileExists = await exists(errorFilePath);
        if (!fileExists) {
            return false; // No error file, backend is fine
        }

        // Read the error file
        try {
            const errorMessage = await readTextFile(errorFilePath);

            if (errorMessage && errorMessage.trim()) {
                log("ERROR", "Backend startup error detected:", errorMessage);

                // Show error to user
                document.body.innerHTML = "";
                const errorDiv = document.createElement("div");
                errorDiv.style.cssText =
                    "padding: 40px; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; color: #f7f7f7; background: rgb(34, 34, 34);";

                const title = document.createElement("h1");
                title.textContent = "AgentMux Startup Error";
                title.style.cssText = "color: #ff6b6b; margin-bottom: 20px;";
                errorDiv.appendChild(title);

                const messagePre = document.createElement("pre");
                messagePre.textContent = errorMessage;
                messagePre.style.cssText = "background: #1a1a1a; padding: 20px; border-radius: 8px; overflow-x: auto;";
                errorDiv.appendChild(messagePre);

                const closeInfo = document.createElement("p");
                closeInfo.textContent = "This window will close in 5 seconds...";
                closeInfo.style.cssText = "margin-top: 20px; color: rgba(255, 255, 255, 0.6);";
                errorDiv.appendChild(closeInfo);

                document.body.appendChild(errorDiv);

                // Close window after 5 seconds
                setTimeout(async () => {
                    try {
                        const window = getCurrentWindow();
                        await window.close();
                    } catch (e) {
                        log("ERROR", "Failed to close window:", e);
                    }
                }, 5000);

                return true; // Error found
            }
        } catch (readError) {
            // File doesn't exist or can't be read - this is normal during startup
            // Backend hasn't failed, it's just still starting
        }

        return false; // No error
    } catch (e) {
        log("WARN", "Error checking for backend startup errors (this is normal if backend not ready):", e);
        return false;
    }
}

async function bootstrap() {
    try {
        log("INFO", "=== Tauri Bootstrap Starting ===");
        log("INFO", "User Agent:", navigator.userAgent);
        log("INFO", "Location:", window.location.href);

        // Dev vs production mode detection
        if (import.meta.env.DEV) {
            console.log("%c[DEV MODE] Loading from Vite dev server — HMR active", "color: lime; font-size: 14px; font-weight: bold");
        } else {
            console.warn("%c[PRODUCTION BUILD] Loading from dist/frontend — source changes will NOT hot-reload!", "color: red; font-size: 14px; font-weight: bold");
        }

        // Check if we're in Tauri
        const isTauriRuntime = typeof (window as any).__TAURI_INTERNALS__ !== "undefined";
        log("INFO", "Is Tauri:", isTauriRuntime);

        if (isTauriRuntime) {
            log("INFO", "Initializing Tauri API...");
            await setupTauriApi();
            log("INFO", "✅ Tauri API initialized successfully");
            log("INFO", "window.api available:", !!(window as any).api);

            // Verify critical methods exist
            const api = (window as any).api;
            log("INFO", "API methods check:");
            log("INFO", "  - getAuthKey:", typeof api?.getAuthKey);
            log("INFO", "  - onContextMenuClick:", typeof api?.onContextMenuClick);
            log("INFO", "  - showContextMenu:", typeof api?.showContextMenu);

            // Check for backend startup errors before loading main app
            log("INFO", "Checking for backend startup errors...");
            const hasError = await checkBackendStartupError();
            if (hasError) {
                log("ERROR", "Backend startup error detected, halting bootstrap");
                return; // Don't load main app if backend failed
            }
            log("INFO", "No backend startup errors detected");
        } else {
            log("INFO", "Not running in Tauri, skipping Tauri init");
        }

        // Now dynamically import wave.ts
        log("INFO", "Loading main application (wave.ts)...");
        try {
            await import("./wave");
            log("INFO", "✅ Main application loaded successfully");
        } catch (waveError) {
            log("ERROR", "Failed to load wave.ts:", waveError);
            log("ERROR", "Wave error name:", (waveError as Error)?.name);
            log("ERROR", "Wave error message:", (waveError as Error)?.message);
            log("ERROR", "Wave error stack:", (waveError as Error)?.stack);
            log("ERROR", "Wave error type:", typeof waveError);
            log("ERROR", "Wave error constructor:", (waveError as any)?.constructor?.name);
            throw waveError;
        }

    } catch (error) {
        log("ERROR", "❌ Bootstrap failed:", error);
        log("ERROR", "Stack:", (error as Error).stack);

        // Show error to user (using DOM methods to avoid XSS)
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

// Capture unhandled errors to backend log for debugging
window.addEventListener("error", (event) => {
    log("UNCAUGHT-ERROR", event.message, "at", event.filename, "line", String(event.lineno));
});
window.addEventListener("unhandledrejection", (event) => {
    log("UNHANDLED-REJECTION", event.reason?.message ?? String(event.reason));
});

// Start bootstrap
bootstrap();
