// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// NOTE: Browser is a pane-level view for embedded web browsing.
// Phase 1 uses an iframe (works for most sites). Phase 2 will add
// native CefBrowserView for sites that block iframes.

import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { getApi } from "@/util/app-api";
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
import { browserStartPageAtom } from "@/store/config-signals";
import { refocusNode } from "@/app/store/block-component-registry";
import { createEffect, createMemo, createRoot, createSignal, onCleanup, type Accessor } from "solid-js";
import { originCovered, pageOrigin } from "./bypass";
import { titleMetaFor } from "./given-title";
import {
    applyDownloadUpdate,
    DONE_ROW_MS,
    dismissDownload,
    type DownloadRow,
    type DownloadUpdate,
} from "./downloads";
import { errorPageFailedUrl } from "./error-page-url";
import { findTabHolding } from "@/app/util/reveal-block";
import { getLayoutModelForStaticTab, getLayoutModelForTabById } from "@/layout/lib/layoutModelHooks";
import { closeBlockInStack, effectiveStack } from "@/layout/lib/layoutStack";
import { MENU_ICON } from "@/app/util/menu-icons";
import { devModeOn } from "@/app/store/dev-mode";
import {
    KEEPAWAKE_META_KEY,
    LOADMS_META_KEY,
    readSleep,
    SLEEP_META_KEY,
    WAKE_META_KEY,
    type BrowserSleep,
    type SleepReason,
} from "./hibernation";
import { recordLoad } from "./load-history";

/** Close `blockId` the way its × would: just its tab when the pane holds
 *  others, else the pane. Looks in the window tab on screen, then in the
 *  window's other tabs: a page can close itself while its tab is in the
 *  background (a sign-in finishing after the user switched tabs). */
export async function closePaneOrTab(blockId: string): Promise<void> {
    let model = getLayoutModelForStaticTab();
    let node = model?.getNodeByBlockId(blockId);
    if (!node) {
        const tabId = await findTabHolding(blockId);
        model = tabId ? getLayoutModelForTabById(tabId) : undefined;
        node = model?.getNodeByBlockId(blockId);
    }
    if (!model || !node) {
        console.warn(`[browser-pane] close requested for ${blockId}, which isn't in a window tab of this window`);
        return;
    }
    if (node.data && effectiveStack(node.data).length > 1) await closeBlockInStack(model, node.id, blockId);
    else await model.closeNode(node.id);
}

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
            // before (typically the address bar). Without blurring it, the
            // shared pane-selection guard (userCaretInBlock) would see the
            // caret still in this block's URL bar and never call
            // `giveFocus()` — leaving OS focus on the main window instead of
            // the pane HWND the user just clicked. See PR #760.
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
 * explicitly. See docs/specs/SPEC_BROWSER_PANE_DEFAULT_URL_AND_POPUP_2026_04_21.md.
 */
export const DEFAULT_BROWSER_URL = "https://agentmux.ai";

/** A meta value that names someone: a non-blank string, trimmed. */
function trimmedString(v: unknown): string | undefined {
    return typeof v === "string" && v.trim() ? v.trim() : undefined;
}

/** The agent the person took a pane from and hasn't given it back to, from the pane's meta. */
function takenFrom(meta: MetaType | undefined): string | undefined {
    return meta?.["browser:given_back"] ? undefined : trimmedString(meta?.["browser:last_driver"]);
}

/** A `browser-pane-nav-state` event: the page's address and history state, from the backend. */
interface NavStatePayload {
    block_id: string;
    url: string;
    can_go_back?: boolean;
    can_go_forward?: boolean;
    url_only?: boolean;
    is_loading?: boolean;
}

/** The browser's state behind its native pane tab (`browserPaneTab`, browser.tsx). */
export class BrowserViewModel {
    viewType = "browser";
    blockId: string;

    viewName!: Accessor<string>;
    viewNameIsPlaceholder!: Accessor<boolean>;
    viewFaviconUrl!: Accessor<string>;
    /** Disposes the createRoot that owns `viewName` and `viewFaviconUrl`.
     *  Called from `dispose()` to release the memos cleanly. */
    private _memoRootDispose: (() => void) | null = null;


    private _url = createSignal<string>("");
    urlAtom: Accessor<string> = this._url[0];
    setUrl = this._url[1];

    private _title = createSignal<string>(TITLE_FALLBACK);
    titleAtom: Accessor<string> = this._title[0];
    setTitle = this._title[1];
    /** Whether `titleAtom` is the page's real title yet (the reducer's
     *  `titleOverridden`), vs. the "Browser" fallback / hostname stand-in
     *  every freshly built ViewModel starts with. */
    private _titleIsReal = createSignal<boolean>(false);

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

    /** Unsubscribe from `browser-pane-close-requested`: the page called
     *  `window.close()`, so the pane closes as its × would. */
    private _closeRequestUnsub: (() => void) | null = null;

    /** Unsubscribe from `browser-pane-title-change` IPC events. */
    private _titleUnsub: (() => void) | null = null;

    /** Unsubscribe from `browser-pane-favicon-urls` IPC events. */
    private _faviconUnsub: (() => void) | null = null;

    /** Unsubscribe from `browser-pane-notice`. */
    private _noticeUnsub: (() => void) | null = null;

    /** The pane's downloads, from the host's `pane-download-update` events
     *  (SPEC_BROWSER_PANE_DOWNLOADS_2026_10_14.md §5). */
    private _downloads = createSignal<DownloadRow[]>([]);
    downloadsAtom: Accessor<DownloadRow[]> = this._downloads[0];
    private _downloadUnsub: (() => void) | null = null;
    /** A finished row folds away after `DONE_ROW_MS`; these are its timers. */
    private _downloadTimers = new Map<string, ReturnType<typeof setTimeout>>();

    /** The host's own notice that a navigation stopped, for when srv couldn't
     *  be told: shown when srv has no banner up, and taken down here. */
    private _localNotice = createSignal<BrowserAttention | undefined>(undefined);

    /** The block's meta — the host context's, reactive. */
    meta: Accessor<MetaType | undefined>;
    private ctx: PaneTabHostContext;
    showControlsAtom!: Accessor<boolean>;
    /** The agent driving this pane, when an agent opened it with `OpenBrowser`
     *  (`browser:owner_agent`, written by srv; SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md §5.5). */
    driverAgentAtom!: Accessor<string | undefined>;
    /** The agent the person took this pane from and has not given it back to: the agent that drove it last
     *  (`browser:last_driver`, written by srv) unless srv recorded a Give back (`browser:given_back`). */
    lastDriverAtom!: Accessor<string | undefined>;
    /** The agent driving the pane did not open it (`browser:took_control`, written by srv). */
    tookControlAtom!: Accessor<boolean>;
    /** The site of the page that opened this pane as a popup
     *  (`browser:popup_from`, written by srv; SPEC_BROWSER_PANE_POPUPS_ADOPTED_2026_10_08.md §5). */
    popupFromAtom!: Accessor<string | undefined>;
    /** The popup windows this pane's page has open (`browser:popup_windows`,
     *  written by srv; SPEC_BROWSER_PANE_NATIVE_POPUPS_AGENT_DRIVEN_2026_10_08.md §7). */
    popupWindowsAtom!: Accessor<{ id: string; url: string }[]>;
    /** The driving agent's pending request to the user (hand-off, approval,
     *  or a navigation off the pane's sites), or a notice of one stopped. */
    attentionAtom!: Accessor<BrowserAttention | undefined>;
    /** The bypass the person granted the driving agent, while it lasts
     *  (`browser:bypass`, written by srv; SPEC_BROWSER_PANE_BYPASS_2026_10_16.md §3.3). */
    bypassAtom!: Accessor<BrowserBypass | undefined>;
    /** What the driving agent did here, newest last (`browser:actions`,
     *  written by srv; PLAN_BROWSER_DRIVER_FIXES_2026_10_17.md §5). */
    actionsAtom!: Accessor<BrowserActions>;
    /** The person paused the driving agent (`browser:paused`, written by srv). */
    pausedAtom!: Accessor<boolean>;
    /** Why srv ended the last bypass on its own (its time ran out, the agent
     *  went quiet or turned it off), shown in the strip until the next one. */
    bypassEndedAtom!: Accessor<string | undefined>;
    /** Now, in ms: ticks while a bypass shows, for its countdown and expiry. */
    private _now = createSignal(Date.now());
    /** The sites the driving agent limited this pane to (`browser:allowed_origins`,
     *  written by srv; SPEC_BROWSER_PANE_ALLOWED_ORIGINS_2026_10_09.md §5). */
    allowedOriginsAtom!: Accessor<string[]>;
    /** The jar this tab browses as (`browser:identity`), fixed when it was
     *  opened; undefined for Personal (SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §5). */
    identityAtom!: Accessor<string | undefined>;
    /** The pane's sleep record while it is asleep or waking (`browser:sleep`;
     *  SPEC_BROWSER_PANE_HIBERNATION_2026_10_20.md §4). */
    sleepAtom!: Accessor<BrowserSleep | undefined>;
    /** How long the page took to load the last time it did (`browser:loadms`), in ms. */
    loadMsAtom!: Accessor<number | undefined>;
    /** The person asked to keep this pane awake (`browser:keepawake`). */
    keepAwakeAtom!: Accessor<boolean>;

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

    /** A splitter of this pane's layout is being dragged (pane size badges
     *  are showing): the native page leaves the badge's strip to the DOM. */
    splitterDragging(): boolean {
        return this.ctx.splitterDragging?.() ?? false;
    }

    // A native pane tab (Pane Tab contract Phase 2c): built by `create(ctx)`
    // (browser.tsx); its own block's meta comes from, and goes to, the host
    // context.
    constructor(ctx: PaneTabHostContext) {
        const blockId = ctx.blockId;
        this.ctx = ctx;
        this.blockId = blockId;
        this.diag(`viewmodel-constructed`);

        this.meta = ctx.meta;

        const ctorMetaUrl = (this.meta()?.["url"] as string | undefined) ?? "";
        console.log(`[browser-pane:diag][${blockId.slice(0, 7)}] ctor meta.url=${JSON.stringify(ctorMetaUrl)}`);

        // Register the pane in the slice's slot store SYNCHRONOUSLY before any IPC
        // subscription can dispatch: the store throws on an unregistered dispatch
        // (silent drops would defeat the audit ring). Re-registering on hot reload
        // resets the state to initial.
        bpRegisterPane(blockId, this.projections());
        installEventSinkOnce();
        this._memoRootDispose = this.createMemos();
        this.subscribeTitleAndFavicon();
        this.subscribeNavState();
        this.subscribePaneEvents();
        this.openInitialPage();
    }

    /** What the slot store writes back to the model's signals, each with its diag line.
     *  `tabs` and `activeTabId` are diagnostic only: the tab strip UI does not exist yet. */
    private projections(): BrowserPaneProjections {
        return {
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
            titleIsReal: (next) => {
                this._titleIsReal[1](next);
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
    }

    /**
     * Wraps the memos in a persistent createRoot owner so they survive re-runs of the
     * effect in block.tsx that constructs the view model: its disposal would freeze them
     * at their last value (the favicon-stuck-at-initial-URL regression of 2026-05-18).
     * Their lifecycle is tied to `dispose()` instead. Returns that disposer.
     */
    private createMemos(): () => void {
        return createRoot((dispose) => {
            this.viewName = createMemo(() => this.titleAtom());
            this.viewNameIsPlaceholder = createMemo(() => !this._titleIsReal[0]());
            this.viewFaviconUrl = createMemo(() => {
                const v = this.faviconUrlAtom();
                this.diag(`vm-favicon-memo-eval value=${JSON.stringify(v)}`);
                return v;
            });
            this.showControlsAtom = createMemo(() => (this.meta()?.["browser:show_controls"] as boolean | undefined) ?? true);
            this.attentionAtom = createMemo(() => {
                const v = this.meta()?.["browser:attention"];
                return v && typeof v === "object" && typeof v.id === "string" ? v : this._localNotice[0]();
            });
            const rawBypass = createMemo((): BrowserBypass | undefined => {
                const v = this.meta()?.["browser:bypass"];
                return v &&
                    typeof v === "object" &&
                    "expires_at" in v &&
                    typeof v.expires_at === "number" &&
                    Array.isArray(v.origins)
                    ? v
                    : undefined;
            });
            this.bypassEndedAtom = createMemo(() => {
                const v = this.meta()?.["browser:bypass"];
                return v && typeof v === "object" && "ended" in v && typeof v.ended === "string" ? v.ended : undefined;
            });
            this.bypassAtom = createMemo(() => {
                const b = rawBypass();
                // Paused, its clock stands still: shown however long the pause.
                return b && (typeof b.paused_left === "number" || b.expires_at > this._now[0]()) ? b : undefined;
            });
            // Tick only while a bypass is shown: its countdown, and the strip
            // going away at expiry even before srv's own update arrives.
            createEffect(() => {
                if (!rawBypass()) return;
                this._now[1](Date.now());
                const t = setInterval(() => this._now[1](Date.now()), 15_000);
                onCleanup(() => clearInterval(t));
            });
            this.actionsAtom = createMemo((): BrowserActions => {
                const v = this.meta()?.["browser:actions"];
                const entries = Array.isArray(v?.entries)
                    ? v.entries.filter((e) => typeof e?.ts === "number" && typeof e?.kind === "string")
                    : [];
                return { count: typeof v?.count === "number" ? v.count : entries.length, entries };
            });
            this.pausedAtom = createMemo(() => this.meta()?.["browser:paused"] === true);
            this.driverAgentAtom = createMemo(() => trimmedString(this.meta()?.["browser:owner_agent"]));
            this.lastDriverAtom = createMemo(() => takenFrom(this.meta()));
            this.tookControlAtom = createMemo(() => this.meta()?.["browser:took_control"] === true);
            this.identityAtom = createMemo(() => {
                const v = this.meta()?.["browser:identity"];
                return typeof v === "string" && v ? v : undefined;
            });
            this.sleepAtom = createMemo(() => readSleep(this.meta()?.[SLEEP_META_KEY]));
            this.loadMsAtom = createMemo(() => {
                const v = this.meta()?.[LOADMS_META_KEY];
                return typeof v === "number" && v > 0 ? v : undefined;
            });
            this.keepAwakeAtom = createMemo(() => this.meta()?.[KEEPAWAKE_META_KEY] === true);
            this.allowedOriginsAtom = createMemo(() => {
                const v = this.meta()?.["browser:allowed_origins"];
                return Array.isArray(v) ? v.filter((o) => typeof o === "string") : [];
            });
            this.popupFromAtom = createMemo(() => {
                const v = this.meta()?.["browser:popup_from"];
                return typeof v === "string" && v.trim() ? v.trim() : undefined;
            });
            this.popupWindowsAtom = createMemo(() => {
                const v = this.meta()?.["browser:popup_windows"];
                return Array.isArray(v)
                    ? v.filter((w) => typeof w?.id === "string" && typeof w?.url === "string")
                    : [];
            });
            return dispose;
        });
    }

    /** The page's title and favicon, as CEF reports them. */
    private subscribeTitleAndFavicon(): void {
        // Subscribe to live title changes fired by CEF's on_title_change.
        // Diag note: log EVERY arrival (pre block-id filter) so a
        // mismatched block_id is observable in muxlog — silent drops
        // were the main blind spot when chasing the favicon/title
        // regression on 2026-05-18.
        void getApi().listen<{ block_id: string; title: string }>(
            "browser-pane-title-change",
            (payload) => {
                // Only the pane the event is for logs it (see the favicon listener below).
                if (payload.block_id !== this.blockId) return;
                this.diag(`title-change arrive title=${JSON.stringify(payload.title)}`);
                if (this.closed) {
                    this.diag(`post-close-event-dropped name=browser-pane-title-change`);
                    return;
                }
                this._dispatch({ type: "TitleChanged", title: payload.title }, "title-change");
            },
        ).then((unsub) => {
            this.diag(`sub-registered name=browser-pane-title-change`);
            if (this.closed) unsub();
            else this._titleUnsub = unsub;
        });

        // Subscribe to real favicon URLs fired by CEF's on_favicon_urlchange.
        void getApi().listen<{ block_id: string; urls: string[] }>(
            "browser-pane-favicon-urls",
            (payload) => {
                // Every pane in the window hears every pane's event: only the
                // pane it is for logs it, or N panes log N lines per event.
                if (payload.block_id !== this.blockId) return;
                this.diag(`favicon-urls arrive count=${payload.urls?.length ?? 0} first=${JSON.stringify(payload.urls?.[0])}`);
                if (this.closed) {
                    this.diag(`post-close-event-dropped name=browser-pane-favicon-urls`);
                    return;
                }
                this._dispatch({ type: "FaviconUrlsReceived", urls: payload.urls }, "favicon-urls");
            },
        ).then((unsub) => {
            this.diag(`sub-registered name=browser-pane-favicon-urls`);
            if (this.closed) unsub();
            else this._faviconUnsub = unsub;
        });
    }

    /**
     * The backend's nav-state events (`on_load_end_browser_pane`,
     * `on_loading_state_change_browser_pane`, `set_pane_main_frame_loading`) are the source
     * of truth for the address bar and back/forward: CEF knows the real history, including
     * in-pane link clicks and popup-intercept redirects.
     */
    private subscribeNavState(): void {
        void getApi().listen<NavStatePayload>("browser-pane-nav-state", (payload) => this.onNavState(payload)).then((unsub) => {
            this.diag(`sub-registered name=browser-pane-nav-state`);
            if (this.closed) unsub();
            else this._navUnsub = unsub;
        });
    }

    private onNavState(payload: NavStatePayload): void {
        if (this.closed) {
            this.diag(`post-close-event-dropped name=browser-pane-nav-state url=${payload.url}`);
            return;
        }
        if (payload.block_id !== this.blockId) return;
        this.diag(
            `nav-state recv url=${JSON.stringify(payload.url)} url_only=${!!payload.url_only} is_loading=${payload.is_loading} can_back=${payload.can_go_back} can_forward=${payload.can_go_forward}`,
        );
        // A page that has not committed anything yet has no address: the host reports "" for it,
        // right after the pane is created. That is not the page settling on an empty address, so it
        // must not clear the tab's address and title or be saved as the pane's.
        const hasUrl = payload.url !== "";
        if (hasUrl) this._dispatch({ type: "UrlConfirmed", url: payload.url }, "nav-state");
        // `url_only` events come from `on_load_end_browser_pane`, before the navigation
        // controller has committed, so their can_go_back / can_go_forward are stale: the
        // authoritative values come from `on_loading_state_change_browser_pane`.
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
        // SPEC_BROWSER_PANE_LOADING_BRAIN_INDICATOR_2026_07_11.md §4.2: `is_loading` is the
        // backend's main-frame loading state (never on the `url_only` load-end events), so it
        // alone drives TabLoadingChanged. Only its value is trusted, not "an event arrived":
        // `on_loading_state_change_browser_pane` also fires on start, commit and back-forward.
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
        }
        // No `else { dispatch LoadFinished }`: the Rust side now emits a dedicated is_loading:false
        // event from the same load-end call as the url_only one, and an immediate loading:false
        // here would bypass dispatchLoadingChanged's debounce hold (layer 3).
        // Persist the real URL to block meta so pane restore lands
        // on the last page, not whatever was passed at create time.
        if (hasUrl) this.ctx.setMeta({ url: payload.url, ...titleMetaFor(this.meta(), payload.url) }).catch(() => {});
    }

    /** Clicks, close requests, notices and downloads of this pane's page. */
    private subscribePaneEvents(): void {
        // Click-to-focus: the pane HWND captures clicks at the Win32 level,
        // so the DOM onMouseDown on `.browser-placeholder` never fires. The
        // backend emits this event directly from its WndProc's WM_LBUTTONDOWN
        // handler (see `browser_pane/hwnd.rs`) using a HWND→block_id map
        // registered at pane creation. We drive refocusNode so the layout
        // marks this block as focused (blue border + keyboard shortcut target).
        void getApi().listen<{ block_id: string }>("browser-pane-clicked", (payload) => {
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

        // The page called `window.close()` (a sign-in's "close this window").
        // The host held the close back, because CEF's own close would take the
        // whole window down (docs/retro/RETRO_BROWSER_PANE_WINDOW_CLOSE_QUITS_APP_2026_10_16.md);
        // close the pane here as its × does, which tears it down safely.
        void getApi().listen<{ block_id: string }>("browser-pane-close-requested", (payload) => {
            if (this.closed || payload.block_id !== this.blockId) return;
            this.diag(`close-requested recv`);
            void closePaneOrTab(this.blockId);
        }).then((unsub) => {
            if (this.closed) unsub();
            else this._closeRequestUnsub = unsub;
        });

        // The host stopped a navigation off the pane's sites and srv couldn't
        // say so: the pane says it itself, rather than sit on a blank page
        // (REPORT_BROWSER_PANE_BLANK_AFTER_BLOCKED_SIGN_IN_REDIRECT_RCA_2026_10_16.md §6.2).
        void getApi().listen<{ block_id: string; url?: string; sites?: string[]; why?: string }>(
            "browser-pane-notice",
            (payload) => {
                if (this.closed || payload.block_id !== this.blockId) return;
                this.diag(`notice recv url=${JSON.stringify(payload.url)}`);
                this._localNotice[1]({
                    id: `local-${Date.now()}`,
                    kind: "notice",
                    local: true,
                    url: payload.url,
                    origin: pageOrigin(payload.url),
                    sites: payload.sites ?? [],
                    why: payload.why,
                });
            },
        ).then((unsub) => {
            if (this.closed) unsub();
            else this._noticeUnsub = unsub;
        });

        // A download this pane's page started: the host says how it goes, and the
        // strip under the address bar shows it. Every pane in the window hears
        // every pane's event; only the pane it is for keeps it.
        void getApi().listen<DownloadUpdate>("pane-download-update", (payload) => {
            if (this.closed || payload.block_id !== this.blockId) return;
            this._downloads[1]((rows) => applyDownloadUpdate(rows, payload));
            if ((payload.state === "done" || payload.state === "cancelled") && payload.download_id != null) {
                const key = String(payload.download_id);
                clearTimeout(this._downloadTimers.get(key));
                this._downloadTimers.set(
                    key,
                    setTimeout(() => this.dismissDownload(key), DONE_ROW_MS),
                );
            }
        }).then((unsub) => {
            if (this.closed) unsub();
            else this._downloadUnsub = unsub;
        });
    }

    /** Opens the first tab on the pane's saved page and navigates to it. */
    private openInitialPage(): void {
        // An empty or missing `url` falls back to the user's start page (`browserStartPageAtom`,
        // "Set as Start Page"), then to DEFAULT_BROWSER_URL. The generic browser widget ships NO
        // `meta.url` so the start page applies; widgets with their own `meta.url` keep it.
        // The atom is safe to read here: app-init awaits the config before anything renders
        // (SPEC_BROWSER_PANE_START_PAGE_2026_09_16.md §3.2/§3.3).
        const meta = this.meta();
        const initialUrl =
            ((meta?.["url"] as string | undefined) ?? "").trim()
            || browserStartPageAtom()
            || DEFAULT_BROWSER_URL;
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
        // Normalize URL. A host error page's data: URI (saved by a pane that
        // was showing one) means the address that failed (error-page-url.ts).
        let normalized = url.trim();
        normalized = errorPageFailedUrl(normalized) ?? normalized;
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
        this.ctx.setMeta({ url: normalized, ...titleMetaFor(this.meta(), normalized) }).catch(() => {});
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
        getApi().browserPanes.goBack(this.blockId).catch(() => {});
    }

    goForward(): void {
        this.diag(`goForward closed=${this.closed}`);
        if (this.closed) return;
        this.cancelPendingLoadingHide();
        this._dispatch({ type: "LoadStarted" }, "goForward");
        getApi().browserPanes.goForward(this.blockId).catch(() => {});
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
        imageUrl?: string;
        selectionText?: string;
        isEditable?: boolean;
        canGoBack?: boolean;
        canGoForward?: boolean;
    }): ContextMenuItem[] {
        const items: ContextMenuItem[] = [
            { label: "Back", icon: MENU_ICON.back, enabled: browserCtx?.canGoBack ?? this.canGoBackAtom(), click: () => this.goBack() },
            {
                label: "Forward",
                icon: MENU_ICON.forward,
                enabled: browserCtx?.canGoForward ?? this.canGoForwardAtom(),
                click: () => this.goForward(),
            },
            {
                label: "Reload",
                icon: MENU_ICON.reload,
                // NOT this.reload() -- that method only does local reducer
                // state tricks (UrlCleared + re-Navigate) and never touches
                // the real native CEF browser view, so it's a no-op against
                // the actual page (reagentx P1 on PR #2599). The toolbar
                // Reload button (browser-nav-bar.tsx) has never called
                // reload() either -- it invokes the backend command
                // directly, which this now matches.
                click: () => {
                    getApi().browserPanes.reload(this.blockId).catch(() => {});
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
                    icon: MENU_ICON.cut,
                    click: () => { getApi().browserPanes.cut(this.blockId).catch(() => {}); },
                });
            }
            if (browserCtx.selectionText) {
                editItems.push({
                    label: "Copy",
                    icon: MENU_ICON.copy,
                    click: () => { getApi().browserPanes.copy(this.blockId).catch(() => {}); },
                });
            }
            if (browserCtx.isEditable) {
                editItems.push({
                    label: "Paste",
                    icon: MENU_ICON.paste,
                    click: () => { getApi().browserPanes.paste(this.blockId).catch(() => {}); },
                });
            }
            items.push({ type: "separator" }, ...editItems);
        }
        items.push(...this.linkAndImageItems(browserCtx));
        items.push(
            { type: "separator" },
            {
                label: "Print",
                icon: MENU_ICON.print,
                click: () => { getApi().browserPanes.print(this.blockId).catch(() => {}); },
            },
        );
        // Developer items: only in dev mode (app:devmode).
        if (devModeOn()) {
            items.push(
                {
                    label: "View Page Source",
                    icon: MENU_ICON.viewSource,
                    click: () => {
                        getApi().browserPanes.viewSource(this.blockId).catch(() => {});
                    },
                },
                {
                    label: "Inspect Element",
                    icon: MENU_ICON.inspect,
                    click: () => {
                        getApi()
                            .browserPanes.inspectElement(this.blockId, browserCtx?.x ?? 0, browserCtx?.y ?? 0)
                            .catch(() => {});
                    },
                },
            );
        }
        return items;
    }

    onError(msg: string): void {
        this.diag(`onError msg=${JSON.stringify(msg)}`);
        this._dispatch({ type: "LoadFailed", reason: msg }, "onError");
    }

    giveFocus(): boolean {
        this.diag(`giveFocus closed=${this.closed}`);
        if (this.closed) return false;
        // "The caret is already in this pane's URL bar -- keep it" is decided
        // before this is ever called, by the shared pane-selection guard
        // (userCaretInBlock in focusManager.giveBlockFocus), scoped to THIS
        // block. A view-local copy of that check used to live here and
        // matched a focused input in ANY pane, so selecting a browser pane by
        // keyboard left keystrokes in the other pane's text box. The URL bar's
        // own main_window_focus IPC comes from block.tsx's handleChildFocus.
        // Here: the user wants the embedded page -- tell the host to move
        // Windows-level keyboard focus to the pane's HWND.
        getApi().browserPanes.focus(this.blockId).catch(() => {});
        return true;
    }

    /** Bring one of this pane's popup windows to the front. */
    showPopup(popupId: string): Promise<void> {
        return getApi().browserPanes.showPopup(popupId);
    }

    /** Close one of this pane's popup windows. */
    closePopup(popupId: string): Promise<void> {
        return getApi().browserPanes.closePopup(popupId);
    }

    /** The human answers the pane's banner (a hand-off, an approval, a
     *  question or a notice). Back is a notice's: the pane goes back, and the
     *  notice comes down as dismissed. */
    resolveAttention(decision: "done" | "approve" | "approve_always" | "approve_ask" | "cancel" | "back"): Promise<void> {
        const a = this.attentionAtom();
        if (!a) return Promise.resolve();
        if (decision === "back") {
            this.goBack();
            decision = "cancel";
        }
        if (a.local) {
            this._localNotice[1](undefined);
            return Promise.resolve();
        }
        // Through the host, never srv directly: srv accepts an answer only
        // from the host, so no agent can approve its own request (§5.4).
        // If srv no longer knows the request (it restarted), it takes the
        // banner down itself; the banner key is srv's alone to write.
        return getApi().approvals.decideBrowserAttention(this.blockId, a.id, decision);
    }

    /** The sites a bypass turned on now would cover: the page's own site and
     *  the pane's site list. srv works out the same and refuses the grant if
     *  they differ from what the person was shown (bypass spec §4.2, §5). */
    bypassOriginsPreview(): string[] {
        const out: string[] = [];
        const own = pageOrigin(this.urlAtom());
        if (own) out.push(own);
        for (const o of this.allowedOriginsAtom()) {
            if (!out.includes(o)) out.push(o);
        }
        return out;
    }

    /** The page is off the bypass's sites: the agent's clicks ask again there. */
    bypassPaused(): boolean {
        const b = this.bypassAtom();
        return !!b && !originCovered(b.origins, this.urlAtom());
    }

    bypassMinutesLeft(): number {
        const b = this.bypassAtom();
        if (!b) return 0;
        const left = typeof b.paused_left === "number" ? b.paused_left : b.expires_at - this._now[0]();
        return Math.max(1, Math.ceil(left / 60_000));
    }

    /** The person's Pause or Resume. Through the host, never srv directly:
     *  srv takes it only from the host, so no agent can resume itself.
     *  Rejects with srv's reason, for the strip to show. */
    setPaused(paused: boolean): Promise<void> {
        const approvals = getApi().approvals;
        if (typeof approvals?.setBrowserPaused !== "function") {
            return Promise.reject(new Error("This build of AgentMux can't pause an agent."));
        }
        return approvals.setBrowserPaused(this.blockId, paused);
    }

    /** The person's Bypass (on, for `minutes`) or End bypass. Through the
     *  host, never srv directly: srv grants a bypass only from the host, so
     *  no agent can grant itself one (bypass spec §5). Rejects with srv's
     *  reason, for the confirmation to show. */
    setBypass(on: boolean, minutes?: number): Promise<void> {
        const approvals = getApi().approvals;
        if (typeof approvals?.setBrowserBypass !== "function") {
            return Promise.reject(new Error("This build of AgentMux can't turn auto-approve on."));
        }
        return approvals.setBrowserBypass(
            this.blockId,
            on ? { on, minutes, agent: this.driverAgentAtom(), origins: this.bypassOriginsPreview() } : { on: false },
        );
    }

    /** The human takes the pane back from the agent driving it: clearing
     *  `browser:owner_agent` ends the agent's control at once, since srv's
     *  owner check needs both its own record and this key
     *  (SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md §5.5). */
    takeOver(): Promise<void> {
        return this.ctx.setMeta({ "browser:owner_agent": null });
    }

    /** The human gives the pane back to the agent they took it from, which lets that agent retake it (`Browser retake`).
     *  Through the host like every answer to a pane's banner: srv takes it from the host only, so no agent can give
     *  itself the pane. srv knows who drove it (`browser:last_driver`). */
    giveBack(): Promise<void> {
        return getApi().approvals.decideBrowserAttention(this.blockId, "", "give_back");
    }

    /** Put the pane to sleep: record what it needs to come back; the view
     *  then discards its page in place, or closes it on an engine without the
     *  Discard API (SPEC_BROWSER_PANE_HIBERNATION_2026_10_20.md §4, §10). */
    hibernate(reason: SleepReason, memoryBytes?: number): Promise<void> {
        const sleep: BrowserSleep = {
            since: Date.now(),
            reason,
            url: this.urlAtom() || ((this.meta()?.["url"] as string | undefined) ?? ""),
            title: this._titleIsReal[0]() ? this.titleAtom() : "",
            favicon: this.faviconUrlAtom(),
            loadMs: this.loadMsAtom(),
            ...(memoryBytes != null && memoryBytes > 0 ? { memoryBytes: Math.round(memoryBytes) } : {}),
        };
        this.diag(`hibernate reason=${reason} url=${JSON.stringify(sleep.url)}`);
        return this.ctx.setMeta({ [SLEEP_META_KEY]: sleep });
    }

    /** The page was discarded in place, so it keeps its history and state. */
    markSleepKept(): Promise<void> {
        const sleep = this.sleepAtom();
        if (!sleep || sleep.kept) return Promise.resolve();
        return this.ctx.setMeta({ [SLEEP_META_KEY]: { ...sleep, kept: true } });
    }

    /** The page behind a "kept" sleep record is gone: closed after all, or
     *  from before an app restart or window reload. Nothing is kept. */
    clearSleepKept(): Promise<void> {
        const sleep = this.sleepAtom();
        if (!sleep?.kept) return Promise.resolve();
        return this.ctx.setMeta({ [SLEEP_META_KEY]: { ...sleep, kept: false } });
    }

    /** The page has loaded again after a wake: the pane is awake. */
    endSleep(loadMs: number | undefined): Promise<void> {
        if (loadMs != null && loadMs > 0) recordLoad(this.sleepAtom()?.url || this.urlAtom(), loadMs);
        return this.ctx.setMeta({
            [SLEEP_META_KEY]: null,
            [WAKE_META_KEY]: null,
            ...(loadMs != null && loadMs > 0 ? { [LOADMS_META_KEY]: Math.round(loadMs) } : {}),
        });
    }

    /** How long the page took to load, for the next wake's estimate. */
    recordLoadMs(loadMs: number): Promise<void> {
        recordLoad(this.urlAtom(), loadMs);
        return this.ctx.setMeta({ [LOADMS_META_KEY]: Math.round(loadMs) });
    }

    /** The rows for a link and a picture under the click: copy the link's address, and save the link or the image. */
    private linkAndImageItems(browserCtx?: { linkUrl?: string; imageUrl?: string }): ContextMenuItem[] {
        const items: ContextMenuItem[] = [];
        const linkUrl = browserCtx?.linkUrl;
        const imageUrl = browserCtx?.imageUrl;
        if (linkUrl) {
            items.push(
                { label: "Copy Link Address", icon: MENU_ICON.copyLink, click: () => { this.copyLink(linkUrl); } },
                { label: "Save link as…", icon: MENU_ICON.saveAs, click: () => { this.saveAs(linkUrl); } },
            );
        }
        if (imageUrl) {
            items.push({ label: "Save image as…", icon: MENU_ICON.saveAs, click: () => { this.saveAs(imageUrl); } });
        }
        return items.length ? [{ type: "separator" }, ...items] : [];
    }

    /** Put a link's address on the clipboard; when nothing can take it (a Linux machine with no clipboard
     *  tool and no page clipboard), the pane says so instead of leaving the person to paste an old value. */
    copyLink(url: string): void {
        clipboardWriteText(url).catch(() => {
            this._localNotice[1]({
                id: `local-${Date.now()}`,
                kind: "notice",
                local: true,
                message: "Couldn't copy the link: this computer has no clipboard tool. Install wl-clipboard (Wayland) or xclip or xsel (X11).",
            });
        });
    }

    /** Save a link or an image of the page: the host starts the download, and its Save As dialog and the strip take over. */
    saveAs(url: string): void {
        getApi().browserPanes.startDownload(this.blockId, url).catch(() => {});
    }

    /** Stop a running download (the strip's Cancel). The host holds the download; it
     *  says "cancelled" back, which is what the strip then shows. */
    cancelDownload(downloadId: number): void {
        getApi().browserPanes.cancelDownload(this.blockId, downloadId).catch(() => {});
    }

    /** Show a finished download in its folder (the strip's Show in folder). `path` is the
     *  host's own, from the "done" event. */
    revealDownload(path: string): void {
        getApi()
            .revealInFileExplorer(path)
            .catch(() => {
                this._downloads[1]((rows) =>
                    rows.map((r) => (r.path === path ? { ...r, reason: "Couldn't open the folder." } : r)),
                );
            });
    }

    dismissDownload(key: string): void {
        clearTimeout(this._downloadTimers.get(key));
        this._downloadTimers.delete(key);
        this._downloads[1]((rows) => dismissDownload(rows, key));
    }

    /** The person asked to keep this pane awake. */
    keepAwake(): Promise<void> {
        return this.ctx.setMeta({ [KEEPAWAKE_META_KEY]: true });
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
        if (this._closeRequestUnsub) {
            this._closeRequestUnsub();
            this._closeRequestUnsub = null;
        }
        if (this._titleUnsub) {
            this._titleUnsub();
            this._titleUnsub = null;
        }
        if (this._faviconUnsub) {
            this._faviconUnsub();
            this._faviconUnsub = null;
        }
        if (this._noticeUnsub) {
            this._noticeUnsub();
            this._noticeUnsub = null;
        }
        if (this._downloadUnsub) {
            this._downloadUnsub();
            this._downloadUnsub = null;
        }
        this._downloadTimers.forEach((t) => clearTimeout(t));
        this._downloadTimers.clear();
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
