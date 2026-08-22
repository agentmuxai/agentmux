// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createMemo, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { invokeCommand, listenEvent } from "@/app/platform/ipc";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { FlyoutMenu } from "@/app/element/flyoutmenu";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { findBookmark, toggleBookmark } from "./browser-bookmarks-logic";
import type { BrowserViewModel } from "./browser-model";

// Compact tag for an Element — used by diag log lines to identify the
// previous/next active element across focus transitions.
function tagElement(el: Element | null): string {
    if (!el) return "null";
    const t = el.tagName?.toLowerCase() ?? "?";
    const cls = (el as HTMLElement).className?.toString().split(/\s+/).find((c) => c) ?? "";
    const id = (el as HTMLElement).id ?? "";
    return `${t}${id ? `#${id}` : ""}${cls ? `.${cls}` : ""}`;
}

/**
 * Address bar + nav buttons (back/forward/reload/go). Owns the address-bar
 * text state and its two-way sync with `model.urlAtom()`, and decides
 * whether to hand a navigation to an already-created pane (via
 * `browser_pane_navigate`) or to create the pane for the first time.
 */
export function BrowserNavBar(props: {
    model: BrowserViewModel;
    windowLabel: string;
    diag: (msg: string) => void;
    paneCreated: () => boolean;
    createPane: (url: string) => Promise<void>;
}): JSX.Element {
    const model = props.model;
    const diag = props.diag;
    const windowLabel = props.windowLabel;

    const [addressBar, setAddressBar] = createSignal(model.urlAtom() || "");
    let addressInputRef: HTMLInputElement | undefined;
    // Reactively mirror the model's URL into the address-bar input whenever
    // CEF reports a navigation via the `browser-pane-nav-state` event (in-
    // pane link clicks, redirects, back/forward, popup-intercept). Without
    // this the input stayed frozen at the last user-submitted text while
    // `model.urlAtom()` advanced, so the address bar diverged from the
    // actual pane URL. Skip while the user is actively editing the input
    // (focused) — otherwise we'd clobber mid-keystroke. Reagent caught this
    // on PR #484 review.
    createEffect(() => {
        const modelUrl = model.urlAtom();
        const focused = document.activeElement === addressInputRef;
        const willUpdate = !focused && modelUrl !== addressBar();
        diag(`sync urlAtom=${JSON.stringify(modelUrl)} addressBar=${JSON.stringify(addressBar())} focused=${focused} willUpdate=${willUpdate}`);
        if (focused) return;
        if (modelUrl !== addressBar()) setAddressBar(modelUrl);
    });

    // Shared tail of "actually go somewhere": update model + address-bar
    // state AND drive the real native CEF pane (via IPC when the pane
    // already exists, or by creating it for the first time). Extracted so
    // the bookmark-click path (below) can't drift from the address-bar
    // submit path the way it did before this was pulled out — a bookmark
    // click used to call only `model.navigate()`, which updates reducer
    // state/block-meta but never told the actual CEF pane to navigate, so
    // the address bar changed while the displayed page didn't (reagentx/
    // Codex review, PR #2730).
    const navigateTo = (url: string) => {
        model.navigate(url);
        setAddressBar(url);

        if (props.paneCreated()) {
            invokeCommand("browser_pane_navigate", {
                block_id: model.blockId,
                url,
            }).catch((e: any) => model.onError(`Navigation failed: ${e}`));
        } else {
            props.createPane(url);
        }
    };

    const handleNavigate = () => {
        const url = addressBar().trim();
        diag(`input-submit value=${JSON.stringify(url)}`);
        if (!url) return;

        let normalized = url;
        if (!normalized.match(/^https?:\/\//i) && !normalized.startsWith("about:")) {
            if (normalized.includes(".") && !normalized.includes(" ")) {
                normalized = `https://${normalized}`;
            } else {
                normalized = `https://www.google.com/search?q=${encodeURIComponent(normalized)}`;
            }
        }

        navigateTo(normalized);
    };

    const handleAddressKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter") {
            e.preventDefault();
            handleNavigate();
        }
    };

    // Ctrl+L (Cmd+L on macOS) is dead by default in a browser pane — CEF
    // intercepts it at the pre-key stage (agentmux-cef/src/client/handlers.rs)
    // since a pane's keystrokes go to the CEF child browser, not this webview,
    // and emits `browser-pane-shortcut` instead of forwarding to the (possibly
    // untrusted) page. See issue #1190.
    onMount(() => {
        let unsub: (() => void) | undefined;
        void listenEvent<{ block_id: string; action: string }>(
            "browser-pane-shortcut",
            (payload) => {
                if (payload.block_id !== model.blockId) return;
                if (payload.action !== "focus-address") return;
                diag(`shortcut-focus-address`);
                // Same OS-focus handoff the address bar's own onMouseDown
                // needs (see its comment above): the pane HWND currently
                // holds OS keyboard focus, so a bare DOM .focus() call
                // wouldn't actually move keystrokes to this input without
                // first reclaiming OS focus for this window.
                invokeCommand("main_window_focus", { window_label: windowLabel }).catch(() => {});
                addressInputRef?.focus();
                addressInputRef?.select();
            }
        ).then((fn) => {
            unsub = fn;
        });
        onCleanup(() => unsub?.());
    });

    // Browser pane bookmarks — a global (shared_dir-backed) flat list, NOT
    // per-pane block meta. See
    // docs/specs/SPEC_BROWSER_PANE_BOOKMARKS_AND_GO_ICON_2026_08_22.md.
    // Deliberately re-fetched on every menu open (not once on mount) rather
    // than kept live via a wave-event subscription — matches this
    // codebase's existing MemoryViewModel/GlobalBrainViewModel precedent
    // ("does not subscribe... the manager is the only writer in
    // practice"). A menu opened before another window's edit lands still
    // shows a stale snapshot until closed and reopened — an accepted v1
    // limitation, not something this component tries to solve.
    const [bookmarks, setBookmarks] = createSignal<BrowserBookmark[]>([]);
    const [bookmarksLoading, setBookmarksLoading] = createSignal(false);
    const [bookmarksError, setBookmarksError] = createSignal<string | null>(null);

    const loadBookmarks = async () => {
        setBookmarksLoading(true);
        setBookmarksError(null);
        try {
            const result = await RpcApi.ListBookmarksCommand(TabRpcClient);
            setBookmarks(result.bookmarks ?? []);
        } catch (e) {
            setBookmarksError(`Failed to load bookmarks: ${(e as Error).message ?? e}`);
        } finally {
            setBookmarksLoading(false);
        }
    };

    // Optimistic write with rollback — the menu already closed by the time
    // this resolves (FlyoutMenu closes on any item click), so a failure
    // needs its own visible surface rather than silently reopening the
    // menu. `bookmarksError` surfaces via the bookmark button itself (title
    // tooltip + a red icon tint, below) rather than the page-load error
    // banner in browser-view.tsx — that banner is `model`'s own
    // `errorAtom`/`LoadFailed` state for page navigation failures, a
    // different concern that gets cleared on the next navigation; a
    // bookmark-save failure shouldn't ride along on that lifecycle. See the
    // spec's "shared_dir can't be resolved" unhappy path — a write that
    // didn't land must not look like it succeeded.
    const persistBookmarks = async (next: BrowserBookmark[]) => {
        const previous = bookmarks();
        setBookmarks(next);
        try {
            await RpcApi.SetBookmarksCommand(TabRpcClient, { bookmarks: next });
        } catch (e) {
            setBookmarks(previous);
            setBookmarksError(`Failed to save bookmarks: ${(e as Error).message ?? e}`);
        }
    };

    // Exact-URL match, not append-only — repeatedly toggling the same page
    // must flip between saved/unsaved, never pile up duplicate entries.
    // The actual add/remove decision is in the pure, directly-unit-tested
    // browser-bookmarks-logic.ts (toggleBookmark/findBookmark) — this
    // component only wires it to the model's live signals and the RPC.
    const currentBookmark = createMemo(() => findBookmark(bookmarks(), model.urlAtom()));

    const toggleCurrentBookmark = () => {
        const url = model.urlAtom();
        if (!url) return;
        void persistBookmarks(
            toggleBookmark(bookmarks(), {
                url,
                title: model.titleAtom(),
                faviconUrl: model.faviconUrlAtom() ?? "",
                newId: () => crypto.randomUUID(),
                now: () => Date.now(),
            }),
        );
    };

    // FlyoutMenu — the same primitive behind the hamburger menu, the widget
    // bar's "More" dropdown, and right-click context menus. Reusing it
    // means the "grows to the bottom of the window, then scrolls
    // internally" behavior (computeMenuPosition's size() middleware,
    // frontend/app/util/menu-position.ts) and edge-flip/click-outside-close
    // come for free — nothing bookmark-specific to build there. `icon`
    // uses a plain FontAwesome name (consistent with every other menu item
    // in the app) rather than a live per-site favicon image — FlyoutMenu's
    // default item renderer only supports FA icon-name strings, and
    // special-casing an `<img>` via `renderMenuItem` would be new,
    // unprecedented surface in a shared component for a purely cosmetic
    // upgrade, not worth it for v1.
    const bookmarkMenuItems = createMemo<MenuItem[]>(() => {
        if (bookmarksLoading()) {
            // No icon here (deliberately): FlyoutMenu's default item
            // renderer only ever applies `fa-solid fa-fw fa-${item.icon}` —
            // there's no way to also pair in `fa-spin` through that field,
            // so a `fa-spinner` icon would render static/non-animating and
            // look broken next to every other spinner in this codebase
            // (which is always fa-spinner + fa-spin together, e.g.
            // swarm-view.tsx:757, toolchain-view.tsx:322). A label-only row
            // for this brief, local-file-read loading state is honest
            // rather than a fake, non-spinning spinner (ReAgent re-review,
            // PR #2730).
            return [{ label: "Loading…" }];
        }
        const items: MenuItem[] = [];
        if (model.urlAtom()) {
            items.push({
                label: currentBookmark() ? "Remove Bookmark" : "Bookmark This Page",
                icon: "star",
                onClick: toggleCurrentBookmark,
            });
        }
        const saved = bookmarks();
        if (saved.length > 0) {
            if (items.length > 0) items.push({ label: "", divider: true });
            for (const b of saved) {
                items.push({
                    label: b.title || b.url,
                    icon: "bookmark",
                    onClick: () => navigateTo(b.url),
                });
            }
        } else if (items.length > 0) {
            items.push({ label: "", divider: true });
            items.push({ label: "No bookmarks yet", icon: "bookmark" });
        } else {
            items.push({ label: "No bookmarks yet", icon: "bookmark" });
        }
        return items;
    });

    return (
        <Show when={model.showControlsAtom()}>
            <div class="browser-nav-bar">
                <button
                    class="browser-nav-btn"
                    disabled={!model.canGoBackAtom()}
                    onClick={() => model.goBack()}
                    title="Back"
                >{"←"}</button>
                <button
                    class="browser-nav-btn"
                    disabled={!model.canGoForwardAtom()}
                    onClick={() => model.goForward()}
                    title="Forward"
                >{"→"}</button>
                <button
                    class="browser-nav-btn"
                    onClick={() => invokeCommand("browser_pane_reload", { block_id: model.blockId }).catch(() => {})}
                    title="Reload"
                >{"↻"}</button>
                <FlyoutMenu
                    items={bookmarkMenuItems()}
                    placement="bottom-start"
                    onOpenChange={(open) => {
                        if (open) void loadBookmarks();
                    }}
                >
                    <button
                        class="browser-nav-btn"
                        title={bookmarksError() ? `Bookmarks: ${bookmarksError()}` : "Bookmarks"}
                    >
                        <i
                            class="fa fa-solid fa-bookmark"
                            classList={{ "browser-bookmark-btn-error": !!bookmarksError() }}
                            aria-hidden="true"
                        />
                    </button>
                </FlyoutMenu>
                <input
                    ref={addressInputRef}
                    class="browser-address-bar"
                    type="text"
                    value={addressBar()}
                    onInput={(e) => setAddressBar(e.currentTarget.value)}
                    onKeyDown={handleAddressKeyDown}
                    onContextMenu={showTextInputContextMenu}
                    onMouseDown={() => {
                        // Fire main_window_focus on mousedown — BEFORE focus
                        // moves — so OS keyboard focus reclaims from the pane
                        // HWND at the start of the click. Without this, the
                        // first click on the address bar after the pane HWND
                        // grabbed OS focus only moves DOM focus; OS focus
                        // stays on the pane HWND and keystrokes route there
                        // instead of reaching React. Subsequent clicks work
                        // because OS focus has already transitioned.
                        //
                        // Buttons in the same nav bar work without this
                        // because CEF/Chromium internally calls SetFocus on
                        // <button> click; <input> doesn't get the same
                        // treatment when the parent webview HWND lacks focus.
                        diag(`input-mousedown value=${JSON.stringify(addressBar())}`);
                        invokeCommand("main_window_focus", { window_label: windowLabel }).catch(() => {});
                    }}
                    onFocus={(e) => {
                        // relatedTarget = the element that LOST focus to us
                        // (or null if focus came from outside the document,
                        // e.g. from the embedded CEF browser pane).
                        // document.activeElement is intentionally NOT logged
                        // here — by the time onFocus fires it's already this
                        // input, so it would only ever read as the input
                        // itself.
                        const related = e.relatedTarget as Element | null;
                        diag(`input-focus value=${JSON.stringify(addressBar())} relatedTarget=${tagElement(related)}`);
                        e.currentTarget.select();
                        // Always fire main_window_focus with window_label —
                        // the IPC misrouting was the root cause of the bounce
                        // (see ipc.rs:424-428). With the correct window_label
                        // the IPC is a no-op when the target window is
                        // already foreground, so it's safe to send on every
                        // legitimate focus event without triggering loops.
                        invokeCommand("main_window_focus", { window_label: windowLabel }).catch(() => {});
                    }}
                    onBlur={(e) => {
                        const next = e.relatedTarget as Element | null;
                        // Microtask-deferred so we can see what landed focus AFTER the blur.
                        queueMicrotask(() => {
                            diag(`input-blur value=${JSON.stringify(addressBar())} relatedTarget=${tagElement(next)} now-active=${tagElement(document.activeElement)}`);
                        });
                    }}
                    placeholder="Enter URL or search..."
                />
                <button class="browser-nav-btn browser-go-btn" onClick={handleNavigate} title="Go">
                    <i class="fa fa-solid fa-arrow-right" aria-hidden="true" />
                </button>
            </div>
        </Show>
    );
}
