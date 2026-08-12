// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { invokeCommand, listenEvent } from "@/app/platform/ipc";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
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

        model.navigate(normalized);
        setAddressBar(normalized);

        if (props.paneCreated()) {
            invokeCommand("browser_pane_navigate", {
                block_id: model.blockId,
                url: normalized,
            }).catch((e: any) => model.onError(`Navigation failed: ${e}`));
        } else {
            props.createPane(normalized);
        }
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
                <button class="browser-nav-btn browser-go-btn" onClick={handleNavigate}>Go</button>
            </div>
        </Show>
    );
}
