// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure reducer for the browser-pane state slice (slice #9, Phases 3a +
 * 3b + 3c + 3d + 3e complete). See
 * `docs/specs/browser-pane-reducer-roadmap.md` and the conventions
 * doc `frontend-reducer-conventions-2026-05-03.md`.
 * This module is the exact mirror of slice #4's reducer pattern
 * (agent-pane-state) — same `update(state, command) → { state, events }`
 * shape, same idempotency rules, same no-throw policy.
 *
 * Invariants enforced:
 *   1. Once `closed` flips true, every subsequent command emits
 *      `post-close-command-dropped` and returns the unchanged state.
 *      `Disposed` itself is idempotent (second dispatch is a no-op).
 *   2. `loading` and `error` are mutually exclusive — at most one of
 *      the two is truthy at any time.
 *   3. `Navigate` always clears any prior `error` (a fresh attempt
 *      supersedes the prior failure) AND updates `state.url` to the
 *      target URL atomically with the loading flip.
 *   4. `LoadFinished` is a no-op if `loading` was already false AND
 *      `error` was already null (nothing to clear; avoids a spurious
 *      load-finished event on a steady-state pane).
 *   5. `HistoryUpdated` is idempotent: if the supplied fields match
 *      the current state, return the unchanged state with no event.
 *      An omitted field leaves its cell alone (the host can emit
 *      partial updates).
 *   6. `TitleChanged` folds empty/whitespace input to
 *      `TITLE_FALLBACK` so `state.title` is never blank.
 *      Idempotent on identical (post-fold) values.
 *   7. `UrlConfirmed` updates `state.url` only — does NOT touch
 *      loading/error/history (those transition via the
 *      LoadFinished/LoadFailed/HistoryUpdated dispatches that
 *      typically accompany the same nav-state IPC event).
 *      Idempotent on identical url.
 *   8. `UrlCleared` sets `state.url = ""` without touching any
 *      other cell except `faviconUrl` (which derives from `url`).
 *      Distinct from `UrlConfirmed { url: "" }` so the audit ring
 *      distinguishes the reload force-reload pattern from a
 *      host-confirmed empty URL. Idempotent (already empty → no-op).
 *   9. `faviconUrl` is a **derived projection** of `url` — every
 *      transition that writes `url` also writes
 *      `faviconUrl = deriveFaviconUrl(url)`. There is no
 *      `FaviconChanged` command; the cell is not an independent
 *      input. Empty / unparseable URL → empty faviconUrl (view falls
 *      back to globe icon).
 */

import {
    BrowserPaneCommand,
    BrowserPaneState,
    ReducerResult,
    TITLE_FALLBACK,
    deriveFaviconUrl,
    deriveTitlePlaceholder,
    sameOriginUrl,
} from "./types";

export function update(
    state: BrowserPaneState,
    command: BrowserPaneCommand,
): ReducerResult {
    if (state.closed && command.type !== "Disposed") {
        return {
            state,
            events: [
                {
                    type: "post-close-command-dropped",
                    commandType: command.type,
                },
            ],
        };
    }

    switch (command.type) {
        case "Navigate": {
            // Optimistic header (SPEC_BROWSER_PANE_OPTIMISTIC_HEADER_2026_05_18):
            // also seed the title with a hostname-based placeholder so
            // the header doesn't show the previous page's title for
            // the 500ms–3s before CEF emits `TitleChanged`. The real
            // title overrides this when it arrives.
            const placeholder = deriveTitlePlaceholder(command.url);
            const title = placeholder !== "" ? placeholder : TITLE_FALLBACK;
            return {
                state: {
                    ...state,
                    loading: true,
                    error: null,
                    url: command.url,
                    faviconUrl: deriveFaviconUrl(command.url),
                    faviconOverridden: false,
                    title,
                },
                events: [{ type: "navigate", url: command.url }],
            };
        }

        case "LoadStarted": {
            return {
                state: { ...state, loading: true, error: null },
                events: [{ type: "load-started" }],
            };
        }

        case "LoadFinished": {
            if (!state.loading && state.error === null) {
                return { state, events: [] };
            }
            return {
                state: { ...state, loading: false, error: null },
                events: [{ type: "load-finished" }],
            };
        }

        case "LoadFailed": {
            return {
                state: { ...state, loading: false, error: command.reason },
                events: [{ type: "load-failed", reason: command.reason }],
            };
        }

        case "HistoryUpdated": {
            const nextBack =
                command.canGoBack !== undefined
                    ? command.canGoBack
                    : state.canGoBack;
            const nextForward =
                command.canGoForward !== undefined
                    ? command.canGoForward
                    : state.canGoForward;
            if (
                nextBack === state.canGoBack &&
                nextForward === state.canGoForward
            ) {
                return { state, events: [] };
            }
            return {
                state: {
                    ...state,
                    canGoBack: nextBack,
                    canGoForward: nextForward,
                },
                events: [
                    {
                        type: "history-updated",
                        canGoBack: nextBack,
                        canGoForward: nextForward,
                    },
                ],
            };
        }

        case "UrlConfirmed": {
            if (state.url === command.url) return { state, events: [] };
            // Origin-aware favicon preservation:
            //
            // - Same-origin URL change (in-page redirect, hash change,
            //   querystring update) keeps the CEF-reported favicon to
            //   avoid the real favicon flashing back to the heuristic
            //   on every nav event (reagent P2 on #876).
            // - Cross-origin navigation resets the override so a stale
            //   favicon from the previous site doesn't carry over —
            //   bug found 2026-05-18, the old code preserved
            //   faviconOverridden across all URL changes, so
            //   navigating from agentmux.ai → bing.com → x.com would
            //   keep bing's favicon when CEF was silent about x.com.
            //   The new derived favicon may still flash briefly, but a
            //   subsequent FaviconUrlsReceived from CEF will replace
            //   it. That's a better default than retaining a foreign
            //   favicon indefinitely.
            const originChanged = sameOriginUrl(state.url, command.url) === false;
            // Race guard (reagent + codex P2 on #905 v1): if
            // `FaviconUrlsReceived` for the destination page beat
            // `UrlConfirmed` here, `state.faviconUrl` is already from
            // the new origin even though `state.url` still points at
            // the previous page. Resetting based purely on
            // `originChanged` would overwrite that real favicon with
            // the derived one. So: also keep the override when the
            // current favicon's origin already matches the incoming
            // URL's origin.
            const faviconAlreadyForNewOrigin = state.faviconOverridden
                && sameOriginUrl(state.faviconUrl, command.url);
            const keepOverride = state.faviconOverridden
                && (!originChanged || faviconAlreadyForNewOrigin);
            const faviconUrl = keepOverride
                ? state.faviconUrl
                : deriveFaviconUrl(command.url);
            const faviconOverridden = keepOverride ? state.faviconOverridden : false;
            // Same logic for the title placeholder: cross-origin nav
            // (in-page link click to a new domain) resets the title
            // to the hostname so the user sees instant feedback. CEF
            // will eventually emit the real title via TitleChanged.
            // Same-origin redirects keep the existing real title to
            // avoid a flash to hostname.
            const title = originChanged
                ? (deriveTitlePlaceholder(command.url) || TITLE_FALLBACK)
                : state.title;
            return {
                state: {
                    ...state,
                    url: command.url,
                    faviconUrl,
                    faviconOverridden,
                    title,
                },
                events: [{ type: "url-confirmed", url: command.url }],
            };
        }

        case "UrlCleared": {
            if (state.url === "") return { state, events: [] };
            return {
                state: { ...state, url: "", faviconUrl: "" },
                events: [{ type: "url-cleared" }],
            };
        }

        case "PaneClicked": {
            // Pure event — no state change. The reducer doesn't track
            // DOM/Win32 focus (catalog rule §3); the view's event
            // sink performs the blur+refocus side-effect when the
            // `pane-clicked` event is delivered. Recording through
            // dispatch lands the click in the audit ring.
            return { state, events: [{ type: "pane-clicked" }] };
        }

        case "TitleChanged": {
            const next = command.title.trim() === "" ? TITLE_FALLBACK : command.title;
            if (next === state.title) return { state, events: [] };
            return {
                state: { ...state, title: next },
                events: [{ type: "title-changed", title: next }],
            };
        }

        case "FaviconUrlsReceived": {
            // Reagent P2 on #876: when CEF reports an empty favicon-URL list
            // (page has no <link rel="icon"> tags), fall back to the
            // heuristic-derived favicon from the page URL rather than ""
            // — matches the types.ts doc comment for FaviconUrlsReceived
            // and the page-navigation paths above that already derive on
            // navigation. The override flag stays false so a later real
            // favicon report can replace it.
            const cefUrl = command.urls[0];
            const next = cefUrl ?? deriveFaviconUrl(state.url);
            const overridden = cefUrl !== undefined;
            if (next === state.faviconUrl && state.faviconOverridden === overridden) {
                return { state, events: [] };
            }
            return {
                state: { ...state, faviconUrl: next, faviconOverridden: overridden },
                events: [{ type: "favicon-urls-received", url: next }],
            };
        }

        case "Disposed": {
            if (state.closed) return { state, events: [] };
            return {
                state: { ...state, closed: true },
                events: [{ type: "disposed" }],
            };
        }
    }
}
