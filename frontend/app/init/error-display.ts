// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Show a startup error message in the DOM so the user never sees
 * an infinite grey/blank screen.
 */
export function showStartupError(message: string): void {
    document.body.style.visibility = "visible";
    document.body.style.opacity = "1";
    document.body.classList.remove("is-transparent");

    // Remove the "Starting AgentMux..." loader
    const loader = document.getElementById("startup-loading");
    if (loader) loader.remove();

    // Show error in the main div
    const main = document.getElementById("main");
    if (main) {
        main.innerHTML = "";
        const errorDiv = document.createElement("div");
        errorDiv.style.cssText = "padding: 40px; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; color: #f7f7f7;";

        const title = document.createElement("h2");
        title.textContent = "AgentMux failed to start";
        title.style.cssText = "color: #ff6b6b; margin-bottom: 16px;";
        errorDiv.appendChild(title);

        const msg = document.createElement("pre");
        msg.textContent = message;
        msg.style.cssText = "background: #1a1a1a; padding: 16px; border-radius: 8px; overflow-x: auto; white-space: pre-wrap; font-size: 13px;";
        errorDiv.appendChild(msg);

        const hint = document.createElement("p");
        hint.textContent = "Press F12 for console details. Try closing and reopening the app.";
        hint.style.cssText = "margin-top: 16px; color: rgba(255,255,255,0.5); font-size: 13px;";
        errorDiv.appendChild(hint);

        main.appendChild(errorDiv);
    }
}
