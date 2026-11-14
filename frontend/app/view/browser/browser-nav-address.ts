// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The state behind the browser nav bar's address bar: the text and its two-way sync with the
 * page's URL, the suggestions with the inline completion, and where Enter, a picked suggestion
 * and the arrow keys lead. The hooks are called in this order from BrowserNavBar, which is also
 * the order their effects are created.
 */

import { createEffect, createSignal, type Accessor } from "solid-js";
import { getApi } from "@/util/app-api";
import { settingsAtom } from "@/store/config-signals";
import type { BrowserViewModel } from "./browser-model";
import { createAddressAutocomplete, inlineCompletion, type AddressAutocomplete } from "./address-autocomplete";
import { createFirstClickSelectAll } from "./address-select-all";

export interface BrowserNavBarProps {
    model: BrowserViewModel;
    windowLabel: string;
    diag: (msg: string) => void;
    paneCreated: () => boolean;
    createPane: (url: string) => Promise<void>;
}

export interface AddressBarState {
    addressBar: Accessor<string>;
    setAddressBar: (value: string) => void;
    /** The address `<input>`: its `ref`, and the element once it is set. */
    bindInput: (el: HTMLInputElement) => void;
    input: () => HTMLInputElement | undefined;
    firstClickSelectAll: ReturnType<typeof createFirstClickSelectAll>;
    autocomplete: AddressAutocomplete;
    /** What the user typed, without an inline completion shown after it. */
    typed: Accessor<string>;
    setTyped: (value: string) => void;
    /** Set by the input's events and read by the inline completion; not reactive. */
    flags: { deleting: boolean; composing: boolean };
    suggestionsOpen: () => boolean;
    showInInput: (value: string) => void;
    abandonSuggestions: () => void;
}

export function useAddressBarState(props: BrowserNavBarProps): AddressBarState {
    const model = props.model;
    const diag = props.diag;

    const [addressBar, setAddressBar] = createSignal(model.urlAtom() || "");
    let addressInputRef: HTMLInputElement | undefined;
    // The click that focuses the bar selects the whole address, as in Chrome.
    const firstClickSelectAll = createFirstClickSelectAll();

    // Chrome's address bar suggestions (address-autocomplete.ts). `typed` is
    // what the user typed, without an inline completion shown after it. The
    // search engine is asked too unless the user turned that off.
    const autocomplete = createAddressAutocomplete(
        (text) =>
            getApi().browserPanes.autocomplete?.(
                model.blockId,
                text,
                settingsAtom()?.["browser:searchsuggestions"] !== false
            ) ?? Promise.resolve(undefined)
    );
    const [typed, setTyped] = createSignal("");
    // `deleting`: the last edit removed text. `composing`: an IME composition (CJK and others) is
    // in progress: its text isn't committed yet, so nothing is asked and nothing is written into
    // the input until it ends (writing a completion mid-composition would break it).
    const flags = { deleting: false, composing: false };
    // The keys act on the list only when it answers what was typed. It stays
    // on screen (`autocomplete.open`) while the next answer is on its way.
    const suggestionsOpen = () => autocomplete.open() && autocomplete.matchesFor() === typed();
    const showInInput = (value: string) => {
        setAddressBar(value);
        if (addressInputRef) addressInputRef.value = value;
    };
    // Focus left the address bar. A suggestion previewed with the arrow keys
    // or completed inline was never opened: show the page's own address again.
    const abandonSuggestions = () => {
        if (addressBar() !== typed() && autocomplete.matches().length > 0) {
            showInInput(model.urlAtom() || typed());
        }
        autocomplete.close();
    };

    // Inline completion: when the engine's default match completes what was
    // typed ("git" -> "github.com"), show the rest selected after the caret,
    // as Chrome does. Not while deleting, and only if the caret is still at
    // the end of exactly what was typed, or a completion carried over from
    // the text before is still selected after it (the input's onInput). A
    // carried one stays until the engine has answered for this text, then
    // gives way to that answer's completion, or to none.
    createEffect(() => {
        const text = typed();
        const matches = autocomplete.matches();
        const matchesFor = autocomplete.matchesFor();
        const inline = inlineCompletion(text, { matches, matchesFor }, flags.deleting);
        const el = addressInputRef;
        if (!el || flags.composing || document.activeElement !== el) return;
        const plain = el.value === text && el.selectionEnd === text.length;
        const carrying =
            el.value.length > text.length &&
            el.value.startsWith(text) &&
            el.selectionStart === text.length &&
            el.selectionEnd === el.value.length;
        const answered = matchesFor === text || matches.length === 0;
        if (!(plain && inline) && !(carrying && answered)) return;
        if (el.value === text + inline) return;
        showInInput(text + inline);
        el.setSelectionRange(text.length, el.value.length);
    });
    // Reactively mirror the model's URL into the address-bar input whenever
    // CEF reports a navigation via the `browser-pane-nav-state` event (in-
    // pane link clicks, redirects, back/forward, popup-intercept). Without
    // this the input stayed frozen at the last user-submitted text while
    // `model.urlAtom()` advanced, so the address bar diverged from the
    // actual pane URL. Skip while the user is actively editing the input
    // (focused) — otherwise we'd clobber mid-keystroke (#484).
    createEffect(() => {
        const modelUrl = model.urlAtom();
        const focused = document.activeElement === addressInputRef;
        const willUpdate = !focused && modelUrl !== addressBar();
        diag(`sync urlAtom=${JSON.stringify(modelUrl)} addressBar=${JSON.stringify(addressBar())} focused=${focused} willUpdate=${willUpdate}`);
        if (focused) return;
        if (modelUrl !== addressBar()) setAddressBar(modelUrl);
    });

    return {
        addressBar,
        setAddressBar,
        bindInput: (el) => {
            addressInputRef = el;
        },
        input: () => addressInputRef,
        firstClickSelectAll,
        autocomplete,
        typed,
        setTyped,
        flags,
        suggestionsOpen,
        showInInput,
        abandonSuggestions,
    };
}

/** What Enter navigates to for typed text: a page as it is, a bare host as https, else a web search. */
function addressToUrl(url: string): string {
    let normalized = url;
    if (!normalized.match(/^https?:\/\//i) && !normalized.startsWith("about:")) {
        if (normalized.includes(".") && !normalized.includes(" ")) {
            normalized = `https://${normalized}`;
        } else {
            normalized = `https://www.google.com/search?q=${encodeURIComponent(normalized)}`;
        }
    }
    return normalized;
}

export interface AddressNavigation {
    navigateTo: (url: string) => void;
    handleAddressKeyDown: (e: KeyboardEvent) => void;
    chooseSuggestion: (m: BrowserAutocompleteMatch) => void;
}

export function useAddressNavigation(props: BrowserNavBarProps, address: AddressBarState): AddressNavigation {
    const model = props.model;
    const diag = props.diag;
    const { autocomplete, typed, flags, suggestionsOpen, showInInput } = address;

    // Shared tail of "actually go somewhere": update model + address-bar
    // state AND drive the real native CEF pane (via IPC when the pane
    // already exists, or by creating it for the first time). Extracted so
    // the bookmark-click path (below) can't drift from the address-bar
    // submit path the way it did before this was pulled out — a bookmark
    // click used to call only `model.navigate()`, which updates reducer
    // state/block-meta but never told the actual CEF pane to navigate, so
    // the address bar changed while the displayed page didn't (#2730).
    const navigateTo = (url: string) => {
        model.navigate(url);
        address.setAddressBar(url);

        if (props.paneCreated()) {
            getApi().browserPanes.navigate(model.blockId, url).catch((e: any) => model.onError(`Navigation failed: ${e}`));
        } else {
            props.createPane(url);
        }
    };

    const handleNavigate = () => {
        const url = address.addressBar().trim();
        diag(`input-submit value=${JSON.stringify(url)}`);
        if (!url) return;

        // The suggestion Enter picks: the highlighted one, else the engine's
        // default match for what was typed (which also knows `localhost:3000`
        // is a URL). Without one, the plain URL-or-search rule below.
        const chosen = autocomplete.chosen(typed());
        autocomplete.close();
        if (chosen?.url) {
            diag(`input-submit chosen=${chosen.type} url=${JSON.stringify(chosen.url)}`);
            navigateTo(chosen.url);
            return;
        }

        navigateTo(addressToUrl(url));
    };

    const handleAddressKeyDown = (e: KeyboardEvent) => {
        // Keys during an IME composition are the IME's (Enter commits it,
        // arrows pick a candidate): never a navigation or a suggestion move.
        if (flags.composing || e.isComposing) return;
        if (e.key === "Enter") {
            e.preventDefault();
            handleNavigate();
            return;
        }
        if (!suggestionsOpen()) return;
        if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            autocomplete.move(e.key === "ArrowDown" ? 1 : -1);
            // Show the highlighted suggestion's text; back to what was typed
            // when the highlight leaves the list.
            const sel = autocomplete.selected();
            const m = sel >= 0 ? autocomplete.matches()[sel] : undefined;
            showInInput(m ? m.fill_into_edit || m.contents : typed());
        } else if (e.key === "Escape") {
            e.preventDefault();
            autocomplete.close();
            showInInput(typed());
        }
    };

    const chooseSuggestion = (m: BrowserAutocompleteMatch) => {
        autocomplete.close();
        if (m.url) navigateTo(m.url);
    };

    return { navigateTo, handleAddressKeyDown, chooseSuggestion };
}
