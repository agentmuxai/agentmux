// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Startup-failure UI + self-heal recovery.
 *
 * When the frontend can't establish `window.api` (the host bridge) the app
 * must not wedge on a dead screen with no way out. The failure is almost
 * always transient: a Vite full-reload mid-change in dev (e.g. a git branch
 * switch in the served clone), a slow backend spawn, or a stale pooled window
 * whose host IPC went away. The backend is usually reachable a moment later,
 * so we self-heal.
 *
 * Mirrors the WebGL-context-loss recovery in `bootstrap.ts`: a bounded,
 * `sessionStorage`-guarded auto-recover (so a persistent failure can't storm —
 * the 2026-06-15 incident reloaded 385×), then a recovery card with a single
 * one-click **Restore**.
 *
 * Recovery re-navigates with the IPC creds **in the URL** rather than calling
 * `location.reload()`. This is the fix for the 2026-06-17 lock-out: on a plain
 * reload the URL carries no creds (cef-init.ts strips them into window globals
 * on first load), so bootstrap depends on the host's `on_load_end`
 * re-injection landing inside `setupCefApi`'s `waitForIpcCreds(2500)` window.
 * When that injection loses the race the reload fails — and EVERY subsequent
 * reload fails the same way, stranding the window on "Can't reconnect". By
 * reading the creds the host already injected (`window.__AGENTMUX_IPC_PORT__` /
 * `__TOKEN__`) and putting them back in the URL, `setupCefApi` reads them
 * synchronously — no race — and bootstrap is deterministic. `windowLabel` is
 * preserved so the restored window rebinds to its SAME workspace.
 *
 * If the in-place heal has already been tried (or no creds are present), Restore
 * escalates: it asks the live host for a brand-new, freshly-bridged window
 * (`open_new_window`, the launcher's path) and closes this dead one — the
 * proven manual recovery, automated. Neither path uses `window.api` (the bridge
 * that failed); both use the lower-level injected creds + `fetch`.
 *
 * Specs: docs/specs/SPEC_HELP_EXTERNAL_LINKS_AND_RESTORE_2026_06_17.md,
 *        docs/specs/SPEC_BRIDGE_INIT_RECOVERY_2026_06_15.md
 */

const RELOAD_KEY = "amux-startup-recover-reloads";
const MAX_RELOADS = 3;
// Set once the manual "Restore" has tried the in-place credentialed re-navigate.
// A second Restore click then escalates to a host-spawned fresh window. Cleared
// on successful startup (clearStartupReloadCount) so a later, unrelated failure
// starts again with the cheap in-place heal.
const RESTORE_TRIED_KEY = "amux-startup-restore-tried";

function readRestoreTried(): boolean {
    try {
        return sessionStorage.getItem(RESTORE_TRIED_KEY) === "1";
    } catch {
        return false;
    }
}

function writeRestoreTried(v: boolean): void {
    try {
        if (v) sessionStorage.setItem(RESTORE_TRIED_KEY, "1");
        else sessionStorage.removeItem(RESTORE_TRIED_KEY);
    } catch {
        // sessionStorage unavailable — escalation just won't latch; harmless.
    }
}

/**
 * Build an app URL that carries the IPC port/token in its query string, read
 * from the globals the host re-injects on every load (`on_load_end`). Loading
 * THIS url lets `cef-init.ts setupCefApi` read the creds synchronously from the
 * URL instead of waiting for the post-load re-injection — closing the
 * `waitForIpcCreds` race that makes a plain `location.reload()` fail to
 * reconnect. Returns null when the creds aren't present (host never injected —
 * genuinely down), so callers can fall back.
 *
 * Built from the CURRENT URL so EVERY existing query param survives — only the
 * two IPC creds are (re)set. `windowLabel` (same-workspace rebind), but also
 * `workspaceId` (torn-off workspace reuse), `floatingPaneId` (floating-pane
 * layout), and `pool=1` (prewarmed pool windows) must be preserved, or a
 * restore in a tear-off / floating / pool window would come back as the wrong
 * window kind. cef-init.ts strips only ipc_port/ipc_token, so the rest persist
 * on `location.search` for us to carry forward.
 *
 * The token rides in the URL only until cef-init.ts strips it back into a global
 * (same handling, and same brief exposure, as the host's own new-window URLs).
 */
function buildCredentialedUrl(): string | null {
    const port = window.__AGENTMUX_IPC_PORT__;
    const token = window.__AGENTMUX_IPC_TOKEN__;
    if (!port || !token) return null;
    try {
        const url = new URL(location.href);
        url.searchParams.set("ipc_port", String(port));
        url.searchParams.set("ipc_token", token);
        return url.toString();
    } catch {
        return null;
    }
}

/**
 * Escalation path: ask the live host to open a brand-new, freshly-bridged window
 * via the same `open_new_window` command the launcher forwards. Uses the
 * re-injected IPC creds directly (NOT `window.api`, which is what failed). The
 * caller closes this dead window afterwards so its workspace is freed for the
 * user to reselect. Returns true if the host accepted the request.
 */
async function spawnFreshWindowViaHost(): Promise<boolean> {
    const port = window.__AGENTMUX_IPC_PORT__;
    const token = window.__AGENTMUX_IPC_TOKEN__;
    if (!port || !token) return false;
    try {
        const resp = await fetch(`http://127.0.0.1:${port}/ipc`, {
            method: "POST",
            headers: {
                "Content-Type": "application/json",
                Authorization: `Bearer ${token}`,
            },
            body: JSON.stringify({ cmd: "open_new_window", args: {} }),
        });
        // The IPC handler returns HTTP 200 even when the command FAILED, encoding
        // the outcome as { success, data, error }. A command-level rejection (e.g.
        // the any_browser_pane_closing() gate in open_window_with_kind) must NOT
        // be read as success — otherwise the caller closes this window with no
        // replacement opened. Require success === true.
        if (!resp.ok) return false;
        const json = (await resp.json().catch(() => null)) as { success?: boolean } | null;
        return json?.success === true;
    } catch {
        return false;
    }
}

/**
 * The single "Restore" action. First press heals THIS window in place
 * (credentialed re-navigate, same workspace, no race). If that was already
 * tried — or there are no creds to use — escalate to a host-spawned fresh window
 * and close this dead one. Last resort (host unreachable): a plain reload.
 */
async function doRestore(): Promise<void> {
    const credUrl = buildCredentialedUrl();
    if (credUrl && !readRestoreTried()) {
        writeRestoreTried(true);
        location.assign(credUrl);
        return;
    }
    if (await spawnFreshWindowViaHost()) {
        window.close();
        return;
    }
    location.reload();
}

function getReloadCount(): number {
    try {
        return parseInt(sessionStorage.getItem(RELOAD_KEY) ?? "0", 10) || 0;
    } catch {
        return 0;
    }
}

function setReloadCount(n: number): void {
    try {
        sessionStorage.setItem(RELOAD_KEY, String(n));
    } catch {
        // sessionStorage unavailable — degrades to no auto-reload (manual card).
    }
}

/**
 * Clear the auto-reload budget. Called ONLY after a successful startup, so a
 * later, unrelated failure begins with a fresh budget. It is deliberately NOT
 * called before a manual reload — resetting there re-entered the auto-reconnect
 * cycle and caused the infinite loop (see tryAutoRecover).
 */
export function clearStartupReloadCount(): void {
    setReloadCount(0);
    writeRestoreTried(false);
}

/**
 * Attempt to self-heal a startup failure.
 *
 * If the bounded reload budget isn't exhausted, show a "Reconnecting…" overlay
 * and reload after a short backoff — returns `true`, meaning the caller should
 * stop (the page is about to reload). Once the budget is exhausted, render the
 * recovery card and return `false` WITHOUT resetting the budget, so any further
 * reload returns to the card instead of re-entering the auto-reconnect loop.
 */
export function tryAutoRecover(message: string): boolean {
    const attempt = getReloadCount() + 1;
    if (attempt <= MAX_RELOADS) {
        setReloadCount(attempt);
        showReconnecting(attempt, MAX_RELOADS);
        // Re-navigate with creds in the URL (deterministic bootstrap, no
        // waitForIpcCreds race) rather than a bare reload that depends on the
        // racy on_load_end re-injection. Falls back to reload() only when the
        // host hasn't injected creds yet (genuinely down). Backoff grows per
        // attempt so a still-churning dev tree gets a moment to settle.
        const credUrl = buildCredentialedUrl();
        setTimeout(() => {
            if (credUrl) location.assign(credUrl);
            else location.reload();
        }, 700 * attempt);
        return true;
    }
    // Budget exhausted — STOP auto-reloading. Do NOT reset the counter: keeping
    // it ≥ MAX means any further reload (including the card's manual Reload) goes
    // straight back to this card instead of re-entering the auto-reconnect cycle.
    // That is what breaks the infinite "keeps trying to reconnect" loop (the bug
    // where Reload reset the budget and looped). The counter is cleared only on a
    // SUCCESSFUL startup (clearStartupReloadCount, from bootstrap's success path).
    showStartupError(message);
    return false;
}

/** True once the bounded auto-reload budget is spent — reloading has not
 *  reconnected, so the card escalates its copy and drops the auto-loop. */
function reloadBudgetExhausted(): boolean {
    return getReloadCount() >= MAX_RELOADS;
}

function mountRoot(): HTMLElement {
    document.body.style.visibility = "visible";
    document.body.style.opacity = "1";
    document.body.classList.remove("is-transparent");
    const loader = document.getElementById("startup-loading");
    if (loader) loader.remove();
    const main = document.getElementById("main") ?? document.body;
    main.innerHTML = "";
    return main;
}

const FONT = "-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif";

/** Transient state shown while an auto-reload is pending. */
function showReconnecting(attempt: number, max: number): void {
    const main = mountRoot();
    const wrap = document.createElement("div");
    wrap.style.cssText =
        "display:flex;flex-direction:column;align-items:center;justify-content:center;height:100vh;gap:12px;" +
        `font-family:${FONT};color:#e8e8e8;background:#101012;`;

    const spinner = document.createElement("div");
    spinner.textContent = "⟳";
    spinner.style.cssText = "font-size:34px;animation:amux-spin 1s linear infinite;";

    const label = document.createElement("div");
    label.textContent = "Reconnecting to AgentMux…";
    label.style.cssText = "font-size:15px;";

    const sub = document.createElement("div");
    sub.textContent = `attempt ${attempt} of ${max}`;
    sub.style.cssText = "font-size:12px;color:rgba(255,255,255,0.5);";

    const style = document.createElement("style");
    style.textContent = "@keyframes amux-spin{to{transform:rotate(360deg)}}";

    wrap.append(spinner, label, sub, style);
    main.appendChild(wrap);
}

/**
 * Terminal recovery card. Friendly by default with a one-click Reload; the raw
 * error + F12 hint live behind a collapsible <details>.
 */
export function showStartupError(message: string): void {
    const main = mountRoot();
    // Once auto-reload has been spent without reconnecting, the failure is
    // likely persistent (a stopped/restarted host), so escalate the copy and
    // point at a real fix rather than implying another reload will help.
    const exhausted = reloadBudgetExhausted();

    const card = document.createElement("div");
    card.style.cssText =
        "max-width:560px;margin:14vh auto 0;padding:28px 32px;border-radius:12px;background:#17181b;" +
        "border:1px solid rgba(255,255,255,0.08);box-shadow:0 8px 40px rgba(0,0,0,0.5);" +
        `font-family:${FONT};color:#e8e8e8;`;

    const title = document.createElement("h2");
    title.textContent = exhausted
        ? "Can't reconnect to AgentMux"
        : "AgentMux lost its connection to the host";
    title.style.cssText = "margin:0 0 10px;font-size:18px;color:#ffd479;";
    card.appendChild(title);

    const body = document.createElement("p");
    body.textContent = exhausted
        ? "Reconnecting on its own didn't work. Press Restore — it rebuilds this " +
          "window's connection to the host and reopens your workspace. If Restore " +
          "can't reach the host either, the host has stopped; restart AgentMux."
        : "The interface loaded but couldn't reach the AgentMux host process. " +
          "This is almost always temporary — Restore reconnects it.";
    body.style.cssText = "margin:0 0 18px;font-size:14px;line-height:1.5;color:rgba(255,255,255,0.8);";
    card.appendChild(body);

    try {
        if (import.meta.env?.DEV) {
            const dev = document.createElement("p");
            dev.textContent =
                "Dev: if you just switched git branches in this clone, Vite reloaded mid-change — Reload should recover.";
            dev.style.cssText = "margin:0 0 18px;font-size:12px;color:rgba(255,255,255,0.45);";
            card.appendChild(dev);
        }
    } catch {
        // import.meta.env unavailable — skip the dev hint.
    }

    const row = document.createElement("div");
    row.style.cssText = "display:flex;gap:10px;margin-bottom:18px;";

    // One button, one job. Restore heals the window in place by re-navigating
    // with creds in the URL (no waitForIpcCreds race, same workspace via the
    // preserved windowLabel); if that was already tried it escalates to a
    // host-spawned fresh window and closes this dead one. See doRestore().
    const restore = document.createElement("button");
    restore.textContent = "⟳ Restore";
    restore.style.cssText =
        "padding:9px 18px;border-radius:7px;border:none;font-size:13px;font-weight:600;" +
        "background:#4c8dff;color:#fff;";
    restore.onclick = () => {
        restore.disabled = true;
        restore.textContent = "Restoring…";
        void doRestore();
    };

    row.append(restore);
    card.appendChild(row);

    const details = document.createElement("details");
    const summary = document.createElement("summary");
    summary.textContent = "Technical details";
    summary.style.cssText = "font-size:12px;color:rgba(255,255,255,0.55);";
    details.appendChild(summary);

    const pre = document.createElement("pre");
    pre.textContent = message;
    pre.style.cssText =
        "margin:10px 0 0;background:#0d0e10;padding:12px;border-radius:7px;overflow-x:auto;" +
        "white-space:pre-wrap;font-size:12px;color:rgba(255,255,255,0.7);";
    details.appendChild(pre);

    const hint = document.createElement("p");
    hint.textContent = "Press F12 for the full console.";
    hint.style.cssText = "margin:8px 0 0;font-size:11px;color:rgba(255,255,255,0.4);";
    details.appendChild(hint);

    card.appendChild(details);
    main.appendChild(card);
}
