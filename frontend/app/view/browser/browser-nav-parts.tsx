// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The parts of the browser nav bar's grid: the navigation buttons with Home, the Bookmarks
 * button and its menu, and the address input with its suggestions.
 */

import { Show, type JSX } from "solid-js";
import clsx from "clsx";
import { getApi } from "@/util/app-api";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { browserStartPageAtom } from "@/store/config-signals";
import { Button, type MenuEntry, MenuTrigger, openMenuAt } from "@/app/element/ui";
import { MENU_ICON } from "@/app/util/menu-icons";
import { ToolbarBookmarkIcon, ToolbarHomeIcon } from "./browser-toolbar-icons";
import { reloadButtonState } from "./browser-reload-button";
import type { BrowserViewModel } from "./browser-model";
import { AddressSuggestions } from "./address-suggestions";
import type { AddressBarState, AddressNavigation, BrowserNavBarProps } from "./browser-nav-address";

// Compact tag for an Element — used by diag log lines to identify the
// previous/next active element across focus transitions.
function tagElement(el: Element | null): string {
    if (!el) return "null";
    const t = el.tagName?.toLowerCase() ?? "?";
    const cls = (el as HTMLElement).className?.toString().split(/\s+/).find((c) => c) ?? "";
    const id = (el as HTMLElement).id ?? "";
    return `${t}${id ? `#${id}` : ""}${cls ? `.${cls}` : ""}`;
}

/** Back, Forward, Reload / Stop and Home. */
export function NavButtons(props: {
    model: BrowserViewModel;
    home: () => string;
    goHome: () => void;
    setCurrentAsStartPage: () => Promise<void>;
}): JSX.Element {
    const model = props.model;
    return (
        <div class="browser-nav-group browser-nav-group--nav">
            <Button
                tone="quiet"
                class="ui-icon-button browser-nav-btn"
                disabled={!model.canGoBackAtom()}
                onClick={() => model.goBack()}
                title="Back"
            >{"←"}</Button>
            <Button
                tone="quiet"
                class="ui-icon-button browser-nav-btn"
                disabled={!model.canGoForwardAtom()}
                onClick={() => model.goForward()}
                title="Forward"
            >{"→"}</Button>
            <Button
                tone="quiet"
                class="ui-icon-button browser-nav-btn"
                data-testid="browser-reload-stop"
                onClick={() => {
                    const panes = getApi().browserPanes;
                    const press = reloadButtonState(model.loadingAtom()).action === "stop" ? panes.stop : panes.reload;
                    press(model.blockId).catch(() => {});
                }}
                title={reloadButtonState(model.loadingAtom()).label}
                aria-label={reloadButtonState(model.loadingAtom()).label}
            >{reloadButtonState(model.loadingAtom()).glyph}</Button>
            <Button
                tone="quiet"
                class="ui-icon-button browser-nav-btn"
                data-testid="browser-home"
                title={`Home: ${props.home()}`}
                aria-label={`Home: ${props.home()}`}
                onClick={props.goHome}
                onContextMenu={(e) => {
                    e.preventDefault();
                    openMenuAt(
                        [
                            {
                                label: "Set this page as Home",
                                icon: MENU_ICON.home,
                                enabled: !!model.urlAtom() && model.urlAtom() !== browserStartPageAtom(),
                                click: () => void props.setCurrentAsStartPage(),
                            },
                        ],
                        e
                    );
                }}
            >
                <ToolbarHomeIcon />
            </Button>
        </div>
    );
}

/** The Bookmarks button and its menu; the button is tinted, with the reason as its tooltip, after a failed load or save. */
export function BookmarksButton(props: {
    items: () => MenuEntry[];
    pane: () => HTMLElement | null | undefined;
    onOpen: () => void;
    bookmarksError: () => string | null;
    startPageError: () => string | null;
}): JSX.Element {
    return (
        <div class="browser-nav-group browser-nav-group--bookmarks">
            <MenuTrigger
                items={props.items()}
                placement="bottom-start"
                containWithin={props.pane}
                alignToAnchorSide
                onOpenChange={(open) => {
                    if (open) props.onOpen();
                }}
            >
                <Button
                    tone="quiet"
                    class="ui-icon-button browser-nav-btn"
                    title={
                        props.bookmarksError()
                            ? `Bookmarks: ${props.bookmarksError()}`
                            : props.startPageError()
                              ? `Home page: ${props.startPageError()}`
                              : "Bookmarks"
                    }
                >
                    <ToolbarBookmarkIcon
                        class={clsx((!!props.bookmarksError() || !!props.startPageError()) && "browser-bookmark-btn-error")}
                    />
                </Button>
            </MenuTrigger>
        </div>
    );
}

/** The address input's event handlers: what is typed, an IME composition, the mouse and focus. */
function addressInputHandlers(bar: BrowserNavBarProps, address: AddressBarState) {
    const diag = bar.diag;
    const windowLabel = bar.windowLabel;
    const { addressBar, setAddressBar, setTyped, autocomplete, flags, firstClickSelectAll } = address;
    return {
        onInput: (e: InputEvent & { currentTarget: HTMLInputElement }) => {
            const value = e.currentTarget.value;
            setAddressBar(value);
            if (flags.composing || e.isComposing) return;
            flags.deleting = e.inputType?.startsWith("delete") ?? false;
            setTyped(value);
            autocomplete.update(value);
        },
        onCompositionStart: () => {
            flags.composing = true;
        },
        onCompositionEnd: (e: CompositionEvent & { currentTarget: HTMLInputElement }) => {
            // The committed text: ask for it now.
            flags.composing = false;
            const value = e.currentTarget.value;
            flags.deleting = false;
            setAddressBar(value);
            setTyped(value);
            autocomplete.update(value);
        },
        onMouseDown: (e: MouseEvent & { currentTarget: HTMLInputElement }) => {
            firstClickSelectAll.onMouseDown(e.currentTarget);
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
            //
            // Known limit: this is a round trip to the host, so
            // on Windows OS focus moves back 11-38 ms after the
            // click, and a key typed sooner (scripted input only)
            // still goes to the page (agentmux#4828). Scripted
            // input should wait ~50 ms after clicking the bar.
            diag(`input-mousedown value=${JSON.stringify(addressBar())}`);
            getApi().reclaimWindowFocus(windowLabel).catch(() => {});
        },
        onFocus: (e: FocusEvent & { currentTarget: HTMLInputElement }) => {
            // relatedTarget = the element that LOST focus to us
            // (or null if focus came from outside the document,
            // e.g. from the embedded CEF browser pane).
            // document.activeElement is intentionally NOT logged
            // here — by the time onFocus fires it's already this
            // input, so it would only ever read as the input
            // itself.
            const related = e.relatedTarget as Element | null;
            diag(`input-focus value=${JSON.stringify(addressBar())} relatedTarget=${tagElement(related)}`);
            firstClickSelectAll.onFocus(e.currentTarget);
            setTyped(e.currentTarget.value);
            // Always fire main_window_focus with window_label —
            // the IPC misrouting was the root cause of the bounce
            // (see the `main_window_focus` arm in ipc.rs). With
            // the correct window_label the IPC is a no-op when
            // the target window is already foreground, so it's safe to send on every
            // legitimate focus event without triggering loops.
            getApi().reclaimWindowFocus(windowLabel).catch(() => {});
        },
        onBlur: (e: FocusEvent) => {
            const next = e.relatedTarget as Element | null;
            address.abandonSuggestions();
            // Microtask-deferred so we can see what landed focus AFTER the blur.
            queueMicrotask(() => {
                diag(`input-blur value=${JSON.stringify(addressBar())} relatedTarget=${tagElement(next)} now-active=${tagElement(document.activeElement)}`);
            });
        },
    };
}

/** The address input with its suggestions list. */
export function AddressInput(props: {
    bar: BrowserNavBarProps;
    address: AddressBarState;
    nav: AddressNavigation;
}): JSX.Element {
    const { addressBar, autocomplete, firstClickSelectAll } = props.address;
    const on = addressInputHandlers(props.bar, props.address);
    return (
        <div class="browser-address-wrap">
            <input
                ref={props.address.bindInput}
                class="browser-address-bar" data-raw-text-field="the address bar: a combobox that takes IME composition, inline completion and the first-click select-all itself, in the nav bar's own row, not a form field" aria-label="Address"
                type="text"
                value={addressBar()}
                role="combobox"
                aria-autocomplete="both"
                aria-expanded={autocomplete.open()}
                onInput={on.onInput}
                onCompositionStart={on.onCompositionStart}
                onCompositionEnd={on.onCompositionEnd}
                onKeyDown={props.nav.handleAddressKeyDown}
                onContextMenu={showTextInputContextMenu}
                onMouseDown={on.onMouseDown}
                onMouseUp={(e) => firstClickSelectAll.onMouseUp(e.currentTarget)}
                onFocus={on.onFocus}
                onBlur={on.onBlur}
                placeholder="Enter URL or search..."
            />
            <Show when={autocomplete.open()}>
                <AddressSuggestions
                    matches={autocomplete.matches()}
                    selected={autocomplete.selected()}
                    onChoose={props.nav.chooseSuggestion}
                />
            </Show>
        </div>
    );
}
