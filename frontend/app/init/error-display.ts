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
 * `sessionStorage`-guarded auto-reload (so a persistent failure can't storm —
 * the 2026-06-15 incident reloaded 385×), then a recovery card with a
 * one-click Reload. `location.reload()` works here because this is a real
 * `http://localhost` document (not the host's `data:` crash page); the IPC
 * port/token are re-supplied by the host's `on_load_end` re-injection on the
 * fresh load — NOT carried in the URL (cef-init.ts strips them into window
 * globals on first load), so reload and "Reopen window" recover identically.
 *
 * Spec: docs/specs/SPEC_BRIDGE_INIT_RECOVERY_2026_06_15.md
 */

const RELOAD_KEY = "amux-startup-recover-reloads";
const MAX_RELOADS = 3;

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
 * Clear the auto-reload budget. Call after a successful startup so a later,
 * unrelated failure begins with a fresh budget (and call before a *manual*
 * reload so the user's click always gets a full set of retries).
 */
export function clearStartupReloadCount(): void {
    setReloadCount(0);
}

/**
 * Attempt to self-heal a startup failure.
 *
 * If the bounded reload budget isn't exhausted, show a "Reconnecting…" overlay
 * and reload after a short backoff — returns `true`, meaning the caller should
 * stop (the page is about to reload). Otherwise reset the budget, render the
 * recovery card, and return `false`.
 */
export function tryAutoRecover(message: string): boolean {
    const attempt = getReloadCount() + 1;
    if (attempt <= MAX_RELOADS) {
        setReloadCount(attempt);
        showReconnecting(attempt, MAX_RELOADS);
        // Backoff grows per attempt so a still-churning dev tree gets a moment
        // to settle between reloads.
        setTimeout(() => location.reload(), 700 * attempt);
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
export function showReconnecting(attempt: number, max: number): void {
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
        ? "Reloading hasn't reconnected to the AgentMux host. The host process may have " +
          "stopped or restarted — close this window and reopen it, or restart AgentMux."
        : "The interface loaded but couldn't reach the AgentMux host process. " +
          "This is almost always temporary — reloading reconnects it.";
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

    const reload = document.createElement("button");
    reload.textContent = "⟳ Reload";
    reload.style.cssText =
        "padding:9px 18px;border-radius:7px;border:none;cursor:pointer;font-size:13px;font-weight:600;" +
        "background:#4c8dff;color:#fff;";
    reload.onclick = () => {
        // Do NOT reset the budget here — that re-entered the auto-reconnect loop.
        // A manual reload is a single attempt: the host re-injects fresh creds on
        // load and invokeCommand retries the 401, so a recovered host succeeds
        // (and bootstrap clears the budget); a still-dead host comes straight back
        // to this card instead of looping the "Reconnecting…" spinner.
        location.reload();
    };

    const reopen = document.createElement("button");
    reopen.textContent = "Reopen window";
    reopen.style.cssText =
        "padding:9px 18px;border-radius:7px;border:1px solid rgba(255,255,255,0.18);cursor:pointer;" +
        "font-size:13px;background:transparent;color:#e8e8e8;";
    reopen.onclick = () => {
        // Re-navigate to the same URL — a fuller reset than reload() (fresh
        // document rather than a reload of the current one). Creds are re-supplied
        // exactly as with reload(): the host's on_load_end re-injection, since the
        // URL no longer carries ipc_port/ipc_token (cef-init.ts strips them on
        // first load). Budget is left intact (cleared only on successful startup).
        location.assign(location.pathname + location.search);
    };

    row.append(reload, reopen);
    card.appendChild(row);

    const details = document.createElement("details");
    const summary = document.createElement("summary");
    summary.textContent = "Technical details";
    summary.style.cssText = "cursor:pointer;font-size:12px;color:rgba(255,255,255,0.55);";
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
