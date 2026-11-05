// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The state behind the nav bar's Bookmarks menu: the saved list, the Home page, and the rows the
 * menu shows. The hooks are called in this order from BrowserNavBar.
 */

import { createMemo, createSignal, Show, type Accessor, type JSX } from "solid-js";
import type { BrowserBookmark } from "@/types/rpc/BrowserBookmark";
import { browserStartPageAtom } from "@/store/config-signals";
import type { MenuEntry } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { findBookmark, toggleBookmark } from "./browser-bookmarks-logic";
import type { BrowserViewModel } from "./browser-model";
import { plainError } from "./plain-error";

/**
 * A saved bookmark's favicon, falling back to the app's existing "no
 * favicon" convention (a plain globe icon, matching
 * `BrowserViewModel.viewIcon`/faviconUrlAtom's own empty-string fallback)
 * when there's no favicon_url, or the image fails to load. A real `<img>`
 * — not a FontAwesome name — since `MenuEntry.icon` is typed
 * `string | JSX.Element` specifically to allow this: the kit Menu draws an
 * element icon as it is, so this brings its own icon slot.
 */
function BookmarkFavicon(props: { faviconUrl: string }): JSX.Element {
    const [failed, setFailed] = createSignal(false);
    return (
        <Show
            when={props.faviconUrl && !failed()}
            fallback={<i class="fa-solid fa-fw fa-globe menu-item-icon" aria-hidden="true" />}
        >
            <img
                src={props.faviconUrl}
                class="menu-item-icon browser-bookmark-favicon"
                onError={() => setFailed(true)}
                alt=""
            />
        </Show>
    );
}

/** Field-by-field equality, not reference equality — used to decide
 *  whether a fresh RPC read actually changed anything worth
 *  re-rendering for (see loadBookmarks below). */
const bookmarkListsEqual = (a: BrowserBookmark[], b: BrowserBookmark[]): boolean =>
    a.length === b.length &&
    a.every((item, i) => {
        const other = b[i];
        return (
            item.id === other.id &&
            item.title === other.title &&
            item.url === other.url &&
            item.favicon_url === other.favicon_url
        );
    });

export interface BookmarkList {
    bookmarks: Accessor<BrowserBookmark[]>;
    bookmarksLoading: Accessor<boolean>;
    /** Why the last load or save failed; null once a later one worked. */
    bookmarksError: Accessor<string | null>;
    loadBookmarks: () => Promise<void>;
    /** The saved bookmark for the page being shown, if any. */
    currentBookmark: Accessor<BrowserBookmark | undefined>;
    toggleCurrentBookmark: () => void;
}

/**
 * Browser pane bookmarks — a global (shared_dir-backed) flat list, NOT
 * per-pane block meta. See
 * docs/specs/SPEC_BROWSER_PANE_BOOKMARKS_AND_GO_ICON_2026_08_22.md.
 * Deliberately re-fetched on every menu open (not once on mount) rather
 * than kept live via a wave-event subscription — matches this
 * codebase's existing BundleViewModel/GlobalBundleViewModel precedent
 * ("does not subscribe... the manager is the only writer in
 * practice"). A menu opened before another window's edit lands still
 * shows a stale snapshot until closed and reopened — an accepted v1
 * limitation, not something this component tries to solve.
 */
export function useBookmarkList(model: BrowserViewModel): BookmarkList {
    const [bookmarks, setBookmarks] = createSignal<BrowserBookmark[]>([]);
    const [bookmarksLoading, setBookmarksLoading] = createSignal(false);
    const [bookmarksError, setBookmarksError] = createSignal<string | null>(null);
    // True once loadBookmarks() has resolved successfully at least once in
    // this pane's lifetime. Gates the "Loading…" placeholder to the very
    // first fetch only — see loadBookmarks' comment below for why later
    // opens must NOT flip bookmarksLoading (#2733).
    const [hasLoadedBookmarksOnce, setHasLoadedBookmarksOnce] = createSignal(false);

    const loadBookmarks = async () => {
        // Only show the transient "Loading…" placeholder before the very
        // first successful load in this pane's lifetime. Every subsequent
        // menu-open refresh keeps rendering the last-known list while the
        // fresh fetch resolves in the background — flipping
        // bookmarksLoading on every open (as this used to, unconditionally)
        // forced bookmarkMenuItems' createMemo to rebuild a fresh
        // MenuEntry[] (fresh <BookmarkFavicon> elements) TWICE per open, and
        // since Solid's <For> (inside the Menu) diffs by reference, that
        // tore down and remounted every saved bookmark's <img> — reloading
        // every favicon over the network on every single menu open, not
        // just the first (#2733).
        if (!hasLoadedBookmarksOnce()) setBookmarksLoading(true);
        setBookmarksError(null);
        try {
            const result = await RpcApi.ListBookmarksCommand(TabRpcClient);
            const next = result.bookmarks ?? [];
            // Skip the signal write entirely when nothing actually
            // changed — keeps the same array/object references so
            // bookmarkMenuItems' memo doesn't rerun and <For> never
            // touches the DOM for unchanged rows. A real change (another
            // window added/removed/edited a bookmark since last open)
            // still updates normally.
            if (!bookmarkListsEqual(bookmarks(), next)) {
                setBookmarks(next);
            }
            setHasLoadedBookmarksOnce(true);
        } catch (e) {
            setBookmarksError(plainError(e, "load your bookmarks", (m) => `Failed to load bookmarks: ${m}`));
        } finally {
            setBookmarksLoading(false);
        }
    };

    // Optimistic write with rollback — the menu already closed by the time
    // this resolves (the Menu closes on any item pick), so a failure
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
            setBookmarksError(plainError(e, "save your bookmarks", (m) => `Failed to save bookmarks: ${m}`));
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

    return { bookmarks, bookmarksLoading, bookmarksError, loadBookmarks, currentBookmark, toggleCurrentBookmark };
}

export interface StartPage {
    /** Why the last save of the Home page failed; null once a later one worked. */
    startPageError: Accessor<string | null>;
    setCurrentAsStartPage: () => Promise<void>;
}

export function useStartPage(model: BrowserViewModel): StartPage {
    // Start page — unlike bookmarks, no local signal/rollback needed: the
    // RPC handler updates the live config and broadcasts it BEFORE
    // responding (crates/srv/src/server/app_api/browser_start_page.rs
    // step 2/3), so by the time this await resolves, fullConfigAtom() — and
    // therefore browserStartPageAtom() — is already current in THIS window
    // and every other open one. See
    // docs/specs/SPEC_BROWSER_PANE_START_PAGE_2026_09_16.md §3.3.
    const [startPageError, setStartPageError] = createSignal<string | null>(null);

    const setCurrentAsStartPage = async () => {
        const url = model.urlAtom();
        if (!url) return;
        // Clear before the attempt, not just on success — a stale error
        // from an earlier failed click must not keep tinting the button red
        // once a later retry actually succeeds (#3288).
        setStartPageError(null);
        try {
            await RpcApi.SetBrowserStartPageCommand(TabRpcClient, { url });
        } catch (e) {
            setStartPageError(plainError(e, "save the Home page", (m) => `Failed to save the Home page: ${m}`));
        }
    };

    return { startPageError, setCurrentAsStartPage };
}

/**
 * The kit Menu (`MenuTrigger`): it grows to the bottom of the window, then scrolls internally
 * (computeMenuPosition's size() middleware, frontend/app/util/menu-position.ts), flips at an edge and
 * closes on an outside press. Saved-bookmark rows use a real per-site favicon (`BookmarkFavicon`, an
 * element that brings its own icon slot) rather than a Font Awesome name; the pinned rows and the
 * placeholder rows use names.
 */
export function useBookmarkMenuItems(
    model: BrowserViewModel,
    list: BookmarkList,
    startPage: StartPage,
    navigateTo: (url: string) => void,
): Accessor<MenuEntry[]> {
    return createMemo<MenuEntry[]>(() => {
        // Only true before the first-ever successful loadBookmarks() call
        // (see its own comment) — every later menu open keeps rendering
        // the previous list below instead of bouncing through this branch,
        // so saved bookmarks' <img> favicons are never torn down and
        // remounted just because the menu was reopened.
        if (list.bookmarksLoading()) {
            // No icon here (deliberately): a spinner needs `fa-spin` beside its name, which an icon name
            // cannot carry, and a static one would look broken next to every other spinner in this
            // codebase. A label-only row for this brief, local-file-read loading state is honest rather
            // than a fake, non-spinning spinner (#2730).
            return [{ label: "Loading…" }];
        }
        const items: MenuEntry[] = [];
        if (model.urlAtom()) {
            // First entry, above the bookmark toggle — see
            // docs/specs/SPEC_BROWSER_PANE_START_PAGE_2026_09_16.md §3.3.
            // Label flips to reflect current state, same convention as the bookmark-toggle row right
            // below (label changes, icon doesn't) — NOT `checked`: that replaces the icon slot with a
            // check-or-blank glyph, which would make the house icon never render at all for this
            // always-boolean row (#3288).
            items.push({
                label: model.urlAtom() === browserStartPageAtom() ? "This Is Your Home Page" : "Set as Home Page",
                icon: "house",
                onSelect: startPage.setCurrentAsStartPage,
            });
            items.push({
                label: list.currentBookmark() ? "Remove Bookmark" : "Bookmark This Page",
                icon: "star",
                onSelect: list.toggleCurrentBookmark,
            });
        }
        const saved = list.bookmarks();
        if (saved.length > 0) {
            if (items.length > 0) items.push({ label: "", divider: true });
            for (const b of saved) {
                items.push({
                    label: b.title || b.url,
                    icon: <BookmarkFavicon faviconUrl={b.favicon_url ?? ""} />,
                    onSelect: () => navigateTo(b.url),
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
}
