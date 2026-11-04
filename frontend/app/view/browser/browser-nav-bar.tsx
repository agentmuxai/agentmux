// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, type JSX } from "solid-js";
import { paneCommandFor } from "@/app/keybindings";
import { browserStartPageAtom } from "@/store/config-signals";
import { DEFAULT_BROWSER_URL } from "./browser-model";
import { BrowserProfileButton, openIncognitoTab } from "./browser-profile-menu";
import { BrowserSigninKey } from "./browser-signin-key";
import { useBookmarkList, useBookmarkMenuItems, useStartPage } from "./browser-bookmarks-menu";
import { useAddressBarState, useAddressNavigation, type BrowserNavBarProps } from "./browser-nav-address";
import { useBrowserPaneShortcuts } from "./browser-nav-shortcuts";
import { AddressInput, BookmarksButton, NavButtons } from "./browser-nav-parts";

/**
 * Address bar + nav buttons (back/forward/reload/go). Owns the address-bar
 * text state and its two-way sync with `model.urlAtom()`, and decides
 * whether to hand a navigation to an already-created pane (via
 * `browser_pane_navigate`) or to create the pane for the first time.
 */
export function BrowserNavBar(props: BrowserNavBarProps): JSX.Element {
    const model = props.model;

    // The bar's menus stay inside the pane when they fit
    // (SPEC_BROWSER_PANE_THIN_BAR_AND_MENUS_IN_PANE_2026_10_18.md §5).
    let barRef: HTMLDivElement | undefined;
    const pane = () => barRef?.closest<HTMLElement>(".browser-view");

    const address = useAddressBarState(props);
    const nav = useAddressNavigation(props, address);

    // Home: the Home page (the start page, SPEC_BROWSER_PANE_START_PAGE), else
    // the default page (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §2).
    const home = () => browserStartPageAtom() || DEFAULT_BROWSER_URL;
    const goHome = () => nav.navigateTo(home());

    useBrowserPaneShortcuts(props, address, home, goHome);

    const list = useBookmarkList(model);
    const startPage = useStartPage(model);
    const bookmarkMenuItems = useBookmarkMenuItems(model, list, startPage, nav.navigateTo);

    return (
        <Show when={model.showControlsAtom()}>
            <div
                ref={barRef}
                class="browser-nav-bar"
                // The bar follows chrome zoom like the pane header above it, so
                // Ctrl+wheel over it zooms chrome too (store/chrome-zoom-target.ts).
                data-zoom-scope="chrome"
                // Ctrl/Cmd+Shift+N anywhere in the bar, the address bar included,
                // opens an Incognito tab, as in Chrome, not the app's new window.
                onKeyDown={(e) => {
                    if (e.defaultPrevented || paneCommandFor(e, "browser") !== "browser:newIncognito") return;
                    e.preventDefault();
                    e.stopPropagation();
                    openIncognitoTab(model, home());
                }}
            >
                {/* One row in a wide pane, two in a thin one (buttons, a line, then the
                    address bar): only the grid's areas change (browser-view.scss), never
                    the DOM, so a resize while typing keeps focus, the IME and the
                    suggestions. Bookmarks is its own cell, just left of Profile at the right
                    end at every width.
                    SPEC_BROWSER_PANE_NAV_BAR_RESPONSIVE_AND_CHROME_ZOOM_2026_10_14.md §3,
                    SPEC_BROWSER_PANE_THIN_BAR_AND_MENUS_IN_PANE_2026_10_18.md §3-§4. */}
                <div class="browser-nav-bar-grid">
                    <NavButtons
                        model={model}
                        home={home}
                        goHome={goHome}
                        setCurrentAsStartPage={startPage.setCurrentAsStartPage}
                    />
                    <BookmarksButton
                        items={bookmarkMenuItems}
                        pane={pane}
                        onOpen={() => void list.loadBookmarks()}
                        bookmarksError={list.bookmarksError}
                        startPageError={startPage.startPageError}
                    />
                    <div class="browser-nav-break" aria-hidden="true" />
                    <AddressInput bar={props} address={address} nav={nav} />
                    <div class="browser-nav-group browser-nav-group--profile">
                        <BrowserSigninKey blockId={model.blockId} meta={() => model.meta()} />
                        <BrowserProfileButton model={model} home={home} containWithin={pane} />
                    </div>
                </div>
            </div>
        </Show>
    );
}
