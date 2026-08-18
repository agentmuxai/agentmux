// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// NOTE: Browser is a pane-level view for embedded web browsing.
// Phase 1 uses an iframe (works for most sites). Phase 2 will add
// native CefBrowserView for sites that block iframes.

import { BlockNodeModel } from "@/app/block/blocktypes";
import { invokeCommand, listenEvent } from "@/app/platform/ipc";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import {
    type BrowserPaneCommand,
    type BrowserPaneEvent,
    TITLE_FALLBACK,
} from "@/app/store/browser-pane-state";
import {
    type BrowserPaneProjections,
    dispatch as bpDispatch,
    registerPane as bpRegisterPane,
    setEventSink as bpSetEventSink,
    snapshot as bpSnapshot,
    unregisterPane as bpUnregisterPane,
} from "@/app/store/browser-pane-state-store";
import { refocusNode } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { createMemo, createRoot, createSignal, type Accessor } from "solid-js";

/**
 * One-time install of the slice #9 event sink. Hands `pane-clicked`
 * (and any future view-effecting events) to the DOM side-effect:
 * blur whatever main-window input held DOM focus, then call
 * `refocusNode(blockId)` so the layout marks the block as focused.
 *
 * This was inline in the `browser-pane-clicked` IPC handler before
 * Phase 4. Routing through the reducer's event sink puts the click
 * in the audit ring (visible in Phase 5's diag panel) without
 * changing the side-effect itself — same blur+refocus, just plumbed
 * differently.
 *
 * Idempotent — checked via a module-level flag because `setEventSink`
 * is a global setter that the last caller wins. We don't want every
 * BrowserViewModel construction to clobber a sink the previous one
 * just installed.
 */
let eventSinkInstalled = false;
function installEventSinkOnce(): void {
    if (eventSinkInstalled) return;
    eventSinkInstalled = true;
    bpSetEventSink((blockId, event: BrowserPaneEvent) => {
        if (event.type === "pane-clicked") {
            // The pane HWND captured this click at Win32 level so React
            // never saw it — `document.activeElement` is whatever it was
            // before (typically the address bar). Without blurring it,
            // the subsequent `giveFocus()` flow sees `isMainInput=true`
            // and tells the host to keep OS focus on the main window —
            // bouncing focus back from the pane HWND we just gave it.
            // See PR #760 for the full diagnosis.
            const active = document.activeElement as HTMLElement | null;
            if (
                active != null &&
                (active.tagName === "INPUT" || active.tagName === "TEXTAREA") &&
                !active.classList.contains("dummy-focus")
            ) {
                console.log(
                    `[browser-pane:diag][${blockId.slice(0, 7)}] pane-click blur active=${active.tagName.toLowerCase()}.${active.className}`,
                );
                active.blur();
            }
            refocusNode(blockId);
        }
    });
}

/**
 * Fallback URL for browser panes created without an explicit `meta.url`.
 * Keeps blank-spawned panes from landing on about:blank (no signposting,
 * no backlink). Callers that want a blank pane can pass `"about:blank"`
 * explicitly. See specs/SPEC_BROWSER_PANE_DEFAULT_URL_AND_POPUP_2026_04_21.md.
 */
const DEFAULT_BROWSER_URL = "https://agentmux.ai";

export class BrowserViewModel implements ViewModel {
    viewType = "browser";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon: Accessor<string> = () => "globe";
    viewName: Accessor<string>;
    viewFaviconUrl: Accessor<string>;
    /** Disposes the createRoot that owns `viewName` and `viewFaviconUrl`.
     *  Called from `dispose()` to release the memos cleanly. */
    private _memoRootDispose: (() => void) | null = null;
    viewText: Accessor<string | HeaderElem[]> = () => [];
    noPadding: Accessor<boolean> = () => true;

    get viewComponent(): ViewComponent {
        return null; // overridden by barrel via Object.defineProperty
    }

    private _url = createSignal<string>("");
    urlAtom: Accessor<string> = this._url[0];
    setUrl = this._url[1];

    private _title = createSignal<string>(TITLE_FALLBACK);
    titleAtom: Accessor<string> = this._title[0];
    setTitle = this._title[1];

    private _favicon = createSignal<string>("");
    /** Pane favicon URL. Derived projection of `state.url` per the
     *  reducer (slice #9 §3e completion). Empty when the URL is
     *  empty or unparseable; the view shows the globe icon in that
     *  case. */
    faviconUrlAtom: Accessor<string> = this._favicon[0];
    private setFavicon = this._favicon[1];

    private _loading = createSignal<boolean>(false);
    loadingAtom: Accessor<boolean> = this._loading[0];
    setLoading = this._loading[1];

    private _canGoBack = createSignal<boolean>(false);
    canGoBackAtom: Accessor<boolean> = this._canGoBack[0];
    setCanGoBack = this._canGoBack[1];

    private _canGoForward = createSignal<boolean>(false);
    canGoForwardAtom: Accessor<boolean> = this._canGoForward[0];
    setCanGoForward = this._canGoForward[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    setError = this._error[1];

    /**
     * Unsubscribe from the backend's `browser-pane-nav-state` event.
     * Nulled in `dispose()` so we don't leak listeners when the pane
     * closes and the ViewModel is GC'd.
     */
    private _navUnsub: (() => void) | null = null;

    /**
     * Unsubscribe from the backend's `browser-pane-clicked` event.
     * Fired when the user clicks inside the pane content (which the
     * DOM never sees because the pane HWND intercepts the click at
     * Win32 level). We use it to drive `refocusNode` so the block
     * shows the blue focus border and keyboard shortcuts target it.
     */
    private _clickUnsub: (() => void) | null = null;

    /** Unsubscribe from `browser-pane-title-change` IPC events. */
    private _titleUnsub: (() => void) | null = null;

    /** Unsubscribe from `browser-pane-favicon-urls` IPC events. */
    private _faviconUnsub: (() => void) | null = null;

    blockAtom: Accessor<Block | undefined>;
    showControlsAtom: Accessor<boolean>;

    /** Late callers (IPC handlers landing post-dispose, defensive guards
     *  in goBack/Forward/reload) read this to no-op instead of firing
     *  IPC against a Browser CEF is mid-destruction. See
     *  docs/specs/SPEC_BROWSER_PANE_LIFECYCLE.md §9 step 4.
     *
     *  Reads the slot store snapshot. Treats unregistered (post-dispose
     *  or never-registered) as `true` so a stale reference never fires
     *  IPC. */
    get closed(): boolean {
        return bpSnapshot(this.blockId)?.closed ?? true;
    }

    /**
     * Wrapper around the slice's slot-store dispatch. Records the `src`
     * tag in a per-call diag log so Phase-1 grep recipes
     * (`muxlog host '\[browser-pane:diag\]'`) keep showing the
     * intent-bearing source ("navigate", "nav-state", "goBack", etc.).
     * The slot store handles state diff + projections + recordDispatch
     * audit — the model just ferries the command in.
     *
     * Guards against double-dispose / post-unregister calls: the slot
     * store throws on unregistered dispatch (no-silent-drops rule),
     * and `dispose()` drops the slot as its final step. Calling
     * `_dispatch` after unregister would throw — same semantics as
     * the reducer's `post-close-command-dropped` event, just at the
     * model layer instead of the reducer. The `closed` snapshot
     * returns `true` for unregistered slots (the `?? true` fallback),
     * so the same check covers both "slot exists with closed=true"
     * and "slot unregistered" cases uniformly.
     */
    private _dispatch(cmd: BrowserPaneCommand, src: string): void {
        // Drop if the slot was already unregistered (post-dispose).
        // `bpDispatch` would throw — same semantics as the reducer's
        // `post-close-command-dropped` event, just enforced at the
        // model layer because the slot is gone entirely.
        if (bpSnapshot(this.blockId) == null) {
            this.diag(`post-close-event-dropped name=${cmd.type} src=${src}`);
            return;
        }
        this.diag(`dispatch type=${cmd.type} src=${src}`);
        bpDispatch(this.blockId, cmd, "system");
    }

    /**
     * Layer 3, SPEC_BROWSER_PANE_LOADING_INDICATOR_FLICKER_2026_08_17.md:
     * cheap defense-in-depth against a rapid true→false→true loading-signal
     * burst that layers 1-2 (the Rust-side main-frame-scoped signal, and
     * the small-badge overlay) didn't catch — e.g. a same-tick redirect
     * hop. NOT a substitute for those; masking symptom without fixing
     * signal fidelity would still let a chatty page wear through this
     * window repeatedly.
     *
     * Only the HIDE direction (`loading: false`) is held. A `true` always
     * dispatches immediately — the spinner should never be slow to SHOW,
     * only avoid flickering back to "loaded" and forth again. Holding a
     * `false` briefly and canceling it if a fresh `true` arrives within the
     * window collapses a flip burst into one visible transition.
     */
    private static readonly LOADING_HIDE_DEBOUNCE_MS = 200;
    private _pendingLoadingHideTimer: ReturnType<typeof setTimeout> | null = null;

    /** Cancel any held `loading:false` dispatch. Called whenever something
     *  else is about to make `loading` true again — a stale held false
     *  firing afterward would otherwise clobber that fresh true back to
     *  false a moment later. */
    private cancelPendingLoadingHide(): void {
        if (this._pendingLoadingHideTimer) {
            clearTimeout(this._pendingLoadingHideTimer);
            this._pendingLoadingHideTimer = null;
        }
    }

    private dispatchLoadingChanged(
        tabId: string,
        loading: boolean,
        canGoBack: boolean,
        canGoForward: boolean,
        src: string,
    ): void {
        this.cancelPendingLoadingHide();
        if (loading) {
            this._dispatch({ type: "TabLoadingChanged", tabId, loading, canGoBack, canGoForward }, src);
            return;
        }
        this._pendingLoadingHideTimer = setTimeout(() => {
            this._pendingLoadingHideTimer = null;
            this._dispatch({ type: "TabLoadingChanged", tabId, loading: false, canGoBack, canGoForward }, src);
        }, BrowserViewModel.LOADING_HIDE_DEBOUNCE_MS);
    }

    /** Tag every diag log with the block prefix so multi-pane sessions
     *  are greppable per pane. See docs/specs/browser-pane-reducer-roadmap.md
     *  Phase 1. */
    // Per-instance ID so we can tell when a stale viewModel reference
    // is hanging around. Used in diag logs alongside blockId. If the
    // setFavicon-side and the memo-read-side print different `vm`
    // values for the same blockId, the bug is "two viewModels for the
    // same blockId, blockframe holds the wrong one."
    public readonly __diagVmId: string = Math.random().toString(36).slice(2, 8);
    private get _diagTag(): string { return `[browser-pane:diag][${this.blockId.slice(0, 7)} vm=${this.__diagVmId}]`; }
    private diag(msg: string): void { console.log(`${this._diagTag} ${msg}`); }

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        this.diag(`viewmodel-constructed`);

        this.blockAtom = getWaveObjectAtom<Block>(makeORef("block", blockId));

        const ctorMetaUrl = (this.blockAtom()?.meta?.["url"] as string | undefined) ?? "";
        console.log(`[browser-pane:diag][${blockId.slice(0, 7)}] ctor meta.url=${JSON.stringify(ctorMetaUrl)}`);

        // Register the pane in the slice's slot store SYNCHRONOUSLY before
        // any IPC subscription can dispatch. The store throws on
        // unregistered dispatch (silent drops would defeat the audit
        // ring); registering here covers every code path that calls
        // `_dispatch` after construction. Re-registering on hot reload
        // is fine — `registerPane` resets the state to initial.
        //
        // Phase 1A note: the slice now holds a tab list. The constructor
        // dispatches `OpenTab(initialUrl)` below so per-tab commands
        // (Navigate, LoadStarted, etc.) have an active tab to mutate;
        // without that, every legacy command would no-op. The new
        // `tabs` + `activeTabId` projections are diagnostic-only here —
        // the tab strip UI lands in Phase 1B and will consume them.
        const projections: BrowserPaneProjections = {
            closed: (next) => {
                this.diag(`state-write key=closed value=${next}`);
                // No view-side signal — `model.closed` reads the slot
                // store snapshot directly. The diag log is the only
                // observable of this projection.
            },
            loading: (next) => {
                this.diag(`state-write key=loading value=${next}`);
                this.setLoading(next);
            },
            error: (next) => {
                this.diag(`state-write key=error value=${JSON.stringify(next)}`);
                this.setError(next);
            },
            tabs: (next) => {
                // Phase 1A: diagnostic-only. Phase 1B's tab strip wires this
                // into a `tabsAtom` Solid signal for rendering.
                this.diag(`state-write key=tabs value-len=${next.length}`);
            },
            activeTabId: (next) => {
                // Phase 1A: diagnostic-only. Phase 1B activates the tab's
                // BrowserView and re-projects per-active-tab fields when
                // this changes.
                this.diag(`state-write key=activeTabId value=${JSON.stringify(next)}`);
            },
            canGoBack: (next) => {
                this.diag(`state-write key=canGoBack value=${next}`);
                this.setCanGoBack(next);
            },
            canGoForward: (next) => {
                this.diag(`state-write key=canGoForward value=${next}`);
                this.setCanGoForward(next);
            },
            title: (next) => {
                this.diag(`state-write key=title value=${JSON.stringify(next)}`);
                this.setTitle(next);
            },
            url: (next) => {
                this.diag(`state-write key=url value=${JSON.stringify(next)}`);
                this.setUrl(next);
            },
            faviconUrl: (next) => {
                this.diag(`state-write key=faviconUrl value=${JSON.stringify(next)}`);
                this.setFavicon(next);
            },
        };
        bpRegisterPane(blockId, projections);
        installEventSinkOnce();

        // Wrap memos in a persistent createRoot owner so they survive
        // re-runs of the surrounding createEffect in block.tsx.
        //
        // block.tsx constructs the viewModel inside a createEffect that
        // re-runs when the meta `view` field changes. The previous
        // run's owner is disposed when the effect re-runs — and that
        // would dispose any createMemo created during the constructor
        // too, even though the viewModel instance is cached and reused.
        // A disposed memo stops subscribing to its signal sources and
        // returns its last cached value forever, which is exactly the
        // favicon-stuck-at-initial-URL regression we chased on
        // 2026-05-18. By hosting the memos in a dedicated root we
        // detach their lifecycle from the surrounding effect and tie
        // them to `dispose()` instead.
        this._memoRootDispose = createRoot((dispose) => {
            this.viewName = createMemo(() => this.titleAtom());
            this.viewFaviconUrl = createMemo(() => {
                const v = this.faviconUrlAtom();
                this.diag(`vm-favicon-memo-eval value=${JSON.stringify(v)}`);
                return v;
            });
            this.showControlsAtom = createMemo(() => (this.blockAtom()?.meta?.["browser:show_controls"] as boolean | undefined) ?? true);
            return dispose;
        });

        // Subscribe to live title changes fired by CEF's on_title_change.
        // Diag note: log EVERY arrival (pre block-id filter) so a
        // mismatched block_id is observable in muxlog — silent drops
        // were the main blind spot when chasing the favicon/title
        // regression on 2026-05-18.
        void listenEvent<{ block_id: string; title: string }>(
            "browser-pane-title-change",
            (payload) => {
                const matched = payload.block_id === this.blockId;
                this.diag(`title-change arrive payload-block=${(payload.block_id ?? "").slice(0, 7)} match=${matched} title=${JSON.stringify(payload.title)}`);
                if (this.closed) {
                    this.diag(`post-close-event-dropped name=browser-pane-title-change`);
                    return;
                }
                if (!matched) return;
                this._dispatch({ type: "TitleChanged", title: payload.title }, "title-change");
            },
        ).then((unsub) => {
            this.diag(`sub-registered name=browser-pane-title-change`);
            if (this.closed) unsub();
            else this._titleUnsub = unsub;
        });

        // Subscribe to real favicon URLs fired by CEF's on_favicon_urlchange.
        void listenEvent<{ block_id: string; urls: string[] }>(
            "browser-pane-favicon-urls",
            (payload) => {
                const matched = payload.block_id === this.blockId;
                this.diag(`favicon-urls arrive payload-block=${(payload.block_id ?? "").slice(0, 7)} match=${matched} count=${payload.urls?.length ?? 0} first=${JSON.stringify(payload.urls?.[0])}`);
                if (this.closed) {
                    this.diag(`post-close-event-dropped name=browser-pane-favicon-urls`);
                    return;
                }
                if (!matched) return;
                this._dispatch({ type: "FaviconUrlsReceived", urls: payload.urls }, "favicon-urls");
            },
        ).then((unsub) => {
            this.diag(`sub-registered name=browser-pane-favicon-urls`);
            if (this.closed) unsub();
            else this._faviconUnsub = unsub;
        });

        // Subscribe to nav-state updates fired by the backend on every
        // `on_load_end_pane`. This is the source of truth for address bar +
        // back/forward state: CEF knows the real history (including
        // in-pane link clicks and popup-intercept redirects), and the
        // local fake history array we used before diverged the moment
        // the user clicked any link inside the pane. See
        // specs/SPEC_BROWSER_PANE_Z_ORDER_2026_04_21.md (unrelated but
        // adjacent) and the nav-state wiring added alongside this PR.
        void listenEvent<{
            block_id: string;
            url: string;
            can_go_back?: boolean;
            can_go_forward?: boolean;
            url_only?: boolean;
            is_loading?: boolean;
        }>("browser-pane-nav-state", (payload) => {
            if (this.closed) {
                this.diag(`post-close-event-dropped name=browser-pane-nav-state url=${payload.url}`);
                return;
            }
            if (payload.block_id !== this.blockId) return;
            this.diag(
                `nav-state recv url=${JSON.stringify(payload.url)} url_only=${!!payload.url_only} is_loading=${payload.is_loading} can_back=${payload.can_go_back} can_forward=${payload.can_go_forward}`,
            );
            this._dispatch({ type: "UrlConfirmed", url: payload.url }, "nav-state");
            // `url_only` events come from `on_load_end_pane` — they arrive
            // before the navigation controller has fully committed, so the
            // `can_go_back` / `can_go_forward` values from that hook would
            // be stale (kimi's investigation identified this race). The
            // authoritative values come from `on_loading_state_change_pane`
            // which CEF invokes with direct params. Skip touching the
            // back/forward atoms on `url_only` events.
            if (
                !payload.url_only &&
                (payload.can_go_back !== undefined ||
                    payload.can_go_forward !== undefined)
            ) {
                this._dispatch(
                    {
                        type: "HistoryUpdated",
                        canGoBack: payload.can_go_back,
                        canGoForward: payload.can_go_forward,
                    },
                    "nav-state",
                );
            }
            // SPEC_BROWSER_PANE_LOADING_BRAIN_INDICATOR_2026_07_11.md §4.2:
            // `is_loading` (present only on `on_loading_state_change_pane`
            // events, never on the `url_only` `on_load_end_pane` ones) is
            // CEF's real navigation-controller loading state — dispatch the
            // reducer's TabLoadingChanged, which was built for exactly this
            // and was never wired up before. This used to unconditionally
            // dispatch LoadFinished on EVERY nav-state event, including ones
            // fired at navigation START — clearing `loading` within the same
            // tick Navigate() had just set it. `on_loading_state_change_pane`
            // fires on start/commit/back-forward too, so only trust its
            // `is_loading` value, not "we received an event at all," as the
            // loading signal.
            if (payload.is_loading !== undefined) {
                const activeTabId = bpSnapshot(this.blockId)?.activeTabId;
                if (activeTabId != null) {
                    this.dispatchLoadingChanged(
                        activeTabId,
                        payload.is_loading,
                        payload.can_go_back ?? this.canGoBackAtom(),
                        payload.can_go_forward ?? this.canGoForwardAtom(),
                        "nav-state",
                    );
                }
            } else {
                // The url_only (on_load_end) event — main-frame load actually
                // finished. Kept as a defense-in-depth clear even though
                // on_loading_state_change_pane's is_loading:false branch
                // above should already have cleared it.
                this._dispatch({ type: "LoadFinished" }, "nav-state");
            }
            // Persist the real URL to block meta so pane restore lands
            // on the last page, not whatever was passed at create time.
            RpcApi.SetMetaCommand(TabRpcClient, {
                oref: makeORef("block", this.blockId),
                meta: { url: payload.url },
            }).catch(() => {});
        }).then((unsub) => {
            this.diag(`sub-registered name=browser-pane-nav-state`);
            if (this.closed) unsub();
            else this._navUnsub = unsub;
        });

        // Click-to-focus: the pane HWND captures clicks at the Win32 level,
        // so the DOM onMouseDown on `.browser-placeholder` never fires. The
        // backend emits this event directly from its WndProc's WM_LBUTTONDOWN
        // handler (see `pane/hwnd.rs`) using a HWND→block_id map registered
        // at pane creation. We drive refocusNode so the layout marks this
        // block as focused (blue border + keyboard shortcut target).
        void listenEvent<{ block_id: string }>("browser-pane-clicked", (payload) => {
            if (this.closed) {
                this.diag(`post-close-event-dropped name=browser-pane-clicked`);
                return;
            }
            if (payload.block_id !== this.blockId) return;
            this.diag(`clicked recv`);
            // Phase 4: route the click through the slice reducer as a
            // `PaneClicked` command. The slot store fires the event
            // sink, which performs the blur-stale-main-input +
            // refocusNode side-effect. Both the dispatch and the
            // event land in the audit ring, so multi-pane focus
            // investigations can see the exact click sequence.
            this._dispatch({ type: "PaneClicked" }, "browser-pane-clicked");
        }).then((unsub) => {
            this.diag(`sub-registered name=browser-pane-clicked`);
            if (this.closed) unsub();
            else this._clickUnsub = unsub;
        });

        // Load URL from block meta on init. An empty/missing `url` falls
        // back to DEFAULT_BROWSER_URL so fresh panes aren't blank (the
        // widget definition in widgets.json also ships this URL, but the
        // fallback covers panes created through the API with no meta.url).
        const meta = this.blockAtom()?.meta;
        const initialUrl = ((meta?.["url"] as string | undefined) ?? "").trim() || DEFAULT_BROWSER_URL;
        // Phase 1A: seed the first tab BEFORE navigate. The reducer's
        // legacy commands (Navigate / LoadStarted / LoadFinished /
        // UrlConfirmed / HistoryUpdated / TitleChanged / FaviconUrlsReceived)
        // all target the active tab; with `tabs: []` and `activeTabId: null`
        // they'd no-op silently. OpenTab creates the tab and sets activeTabId.
        // Then `this.navigate(initialUrl)` runs as before — sets loading=true
        // on the active tab and emits the `navigate` event for the saga
        // (Phase 1C consumer) plus the legacy IPC path the view still uses.
        this._dispatch({ type: "OpenTab", url: initialUrl }, "ctor-open-initial-tab");
        this.navigate(initialUrl);
    }

    navigate(url: string): void {
        this.diag(`navigate(url=${JSON.stringify(url)}) closed=${this.closed}`);
        if (this.closed) return;
        // Normalize URL
        let normalized = url.trim();
        if (!normalized) return;
        if (!normalized.match(/^https?:\/\//i) && !normalized.startsWith("about:")) {
            if (normalized.includes(".") && !normalized.includes(" ")) {
                normalized = `https://${normalized}`;
            } else {
                // Treat as search query
                normalized = `https://www.google.com/search?q=${encodeURIComponent(normalized)}`;
            }
        }

        this.cancelPendingLoadingHide();
        this._dispatch({ type: "Navigate", url: normalized }, "navigate");
        // can_go_back / can_go_forward are set by the `browser-pane-nav-state`
        // event subscription; we don't touch them here. CEF is the source of
        // truth for history state.

        // Persist the requested URL to block meta immediately so a quick
        // pane restore before load_end still has something. The nav-state
        // event will overwrite this with the post-redirect final URL.
        RpcApi.SetMetaCommand(TabRpcClient, {
            oref: makeORef("block", this.blockId),
            meta: { url: normalized },
        }).catch(() => {});
    }

    goBack(): void {
        this.diag(`goBack closed=${this.closed}`);
        if (this.closed) return;
        // Cancel any held loading:false from dispatchLoadingChanged — it
        // would otherwise fire after this dispatch and clobber the fresh
        // loading:true back to false a moment later.
        this.cancelPendingLoadingHide();
        // CEF owns the history — we just fire the IPC. The button's
        // enabled/disabled state came from `can_go_back` in the nav-state
        // event, so if we got here the browser has somewhere to go.
        this._dispatch({ type: "LoadStarted" }, "goBack");
        invokeCommand("browser_pane_go_back", { block_id: this.blockId }).catch(() => {});
    }

    goForward(): void {
        this.diag(`goForward closed=${this.closed}`);
        if (this.closed) return;
        this.cancelPendingLoadingHide();
        this._dispatch({ type: "LoadStarted" }, "goForward");
        invokeCommand("browser_pane_go_forward", { block_id: this.blockId }).catch(() => {});
    }

    reload(): void {
        this.diag(`reload closed=${this.closed}`);
        if (this.closed) return;
        this.cancelPendingLoadingHide();
        const url = this.urlAtom();
        if (url) {
            this._dispatch({ type: "UrlCleared" }, "reload-clear");
            // Force iframe reload by briefly clearing then re-setting
            requestAnimationFrame(() => {
                if (this.closed) return;
                this._dispatch({ type: "Navigate", url }, "reload-restore");
            });
        }
    }

    /**
     * Browser-pane-specific items for the unified pane context menu
     * (SPEC_BROWSER_PANE_UNIFIED_CONTEXT_MENU_2026_08_15.md). `browserCtx` is
     * only populated for the synthetic menu path (blockframe.tsx, driven by
     * the backend's `browser-pane-context-menu` event) — a browser pane has
     * no real DOM `contextmenu` event to call this without it.
     */
    getBodyContextMenuItems(browserCtx?: {
        x?: number;
        y?: number;
        linkUrl?: string;
        selectionText?: string;
        isEditable?: boolean;
        canGoBack?: boolean;
        canGoForward?: boolean;
    }): ContextMenuItem[] {
        const items: ContextMenuItem[] = [
            { label: "Back", enabled: browserCtx?.canGoBack ?? this.canGoBackAtom(), click: () => this.goBack() },
            {
                label: "Forward",
                enabled: browserCtx?.canGoForward ?? this.canGoForwardAtom(),
                click: () => this.goForward(),
            },
            {
                label: "Reload",
                // NOT this.reload() -- that method only does local reducer
                // state tricks (UrlCleared + re-Navigate) and never touches
                // the real native CEF browser view, so it's a no-op against
                // the actual page (reagentx P1 on PR #2599). The toolbar
                // Reload button (browser-nav-bar.tsx) has never called
                // reload() either -- it invokes the backend command
                // directly, which this now matches.
                click: () => {
                    invokeCommand("browser_pane_reload", { block_id: this.blockId }).catch(() => {});
                },
            },
        ];
        // Suppressing CEF's native menu also removed its built-in Cut/Copy/
        // Paste for a text selection or an editable web form field, with
        // nothing replacing them (reagentx P2 on PR #2599). `Frame::copy/
        // cut/paste` on the host operate on whatever is currently selected/
        // focused, same as the native commands would have.
        if (browserCtx?.selectionText || browserCtx?.isEditable) {
            const editItems: ContextMenuItem[] = [];
            if (browserCtx.isEditable && browserCtx.selectionText) {
                editItems.push({
                    label: "Cut",
                    click: () => { invokeCommand("browser_pane_cut", { block_id: this.blockId }).catch(() => {}); },
                });
            }
            if (browserCtx.selectionText) {
                editItems.push({
                    label: "Copy",
                    click: () => { invokeCommand("browser_pane_copy", { block_id: this.blockId }).catch(() => {}); },
                });
            }
            if (browserCtx.isEditable) {
                editItems.push({
                    label: "Paste",
                    click: () => { invokeCommand("browser_pane_paste", { block_id: this.blockId }).catch(() => {}); },
                });
            }
            items.push({ type: "separator" }, ...editItems);
        }
        if (browserCtx?.linkUrl) {
            items.push(
                { type: "separator" },
                {
                    label: "Copy Link Address",
                    click: () => { void clipboardWriteText(browserCtx.linkUrl!); },
                },
            );
        }
        items.push(
            { type: "separator" },
            {
                label: "Print",
                click: () => { invokeCommand("browser_pane_print", { block_id: this.blockId }).catch(() => {}); },
            },
            {
                label: "View Page Source",
                click: () => {
                    invokeCommand("browser_pane_view_source", { block_id: this.blockId }).catch(() => {});
                },
            },
            {
                label: "Inspect Element",
                click: () => {
                    invokeCommand("browser_pane_inspect_element", {
                        block_id: this.blockId,
                        x: browserCtx?.x ?? 0,
                        y: browserCtx?.y ?? 0,
                    }).catch(() => {});
                },
            },
        );
        return items;
    }

    onError(msg: string): void {
        this.diag(`onError msg=${JSON.stringify(msg)}`);
        this._dispatch({ type: "LoadFailed", reason: msg }, "onError");
    }

    giveFocus(): boolean {
        this.diag(`giveFocus closed=${this.closed}`);
        if (this.closed) return false;
        // If a main-window input inside this block (e.g. the URL bar) is
        // already focused, keep it — the user is interacting with the block's
        // chrome, not the embedded page. Also tell the host to move OS-level
        // keyboard focus back to the main window, in case a pane was holding
        // it (otherwise keystrokes still get routed to the pane's HWND).
        const active = document.activeElement as HTMLElement | null;
        const isMainInput =
            active != null &&
            (active.tagName === "INPUT" || active.tagName === "TEXTAREA") &&
            !active.classList.contains("dummy-focus");
        if (isMainInput) {
            invokeCommand("main_window_focus", {}).catch(() => {});
            return true;
        }
        // Otherwise the user wants to interact with the embedded page — tell
        // the host to move Windows-level keyboard focus to the pane's HWND.
        invokeCommand("browser_pane_focus", { block_id: this.blockId }).catch(() => {});
        return true;
    }

    dispose(): void {
        this.diag(`dispose`);
        this.cancelPendingLoadingHide();
        this._dispatch({ type: "Disposed" }, "dispose");
        if (this._navUnsub) {
            this._navUnsub();
            this._navUnsub = null;
        }
        if (this._clickUnsub) {
            this._clickUnsub();
            this._clickUnsub = null;
        }
        if (this._titleUnsub) {
            this._titleUnsub();
            this._titleUnsub = null;
        }
        if (this._faviconUnsub) {
            this._faviconUnsub();
            this._faviconUnsub = null;
        }
        if (this._memoRootDispose) {
            this._memoRootDispose();
            this._memoRootDispose = null;
        }
        // Drop the slot AFTER the Disposed dispatch — the projection
        // for `closed:true` runs first, so any consumer reading
        // `model.closed` mid-dispose still sees true. The unregister
        // is the final step so future post-dispose `_dispatch` calls
        // will throw a clear "unregistered pane" error rather than
        // silently no-oping (the slot store's no-silent-drops rule).
        bpUnregisterPane(this.blockId);
    }
}
