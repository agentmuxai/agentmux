// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { invokeCommand } from "@/app/platform/ipc";
import type { BrowserViewModel } from "./browser-model";
import "./browser-view.scss";

// Compact tag for an Element — used by diag log lines to identify the
// previous/next active element across focus transitions. Module-scope
// so onFocus and onBlur share one implementation.
function tagElement(el: Element | null): string {
    if (!el) return "null";
    const t = el.tagName?.toLowerCase() ?? "?";
    const cls = (el as HTMLElement).className?.toString().split(/\s+/).find((c) => c) ?? "";
    const id = (el as HTMLElement).id ?? "";
    return `${t}${id ? `#${id}` : ""}${cls ? `.${cls}` : ""}`;
}

export function BrowserViewComponent(props: ViewComponentProps<BrowserViewModel>): JSX.Element {
    const model = props.model;
    const _diagTag = `[browser-pane:diag][${model.blockId.slice(0, 7)}]`;
    const diag = (msg: string): void => { console.log(`${_diagTag} ${msg}`); };
    // Captured once at mount — same window_label that createPane uses
    // for browser_pane_create. Required so main_window_focus reclaims
    // OS focus to the WINDOW that sent the IPC (otherwise the host's
    // handler defaults to "main" and misroutes in multi-window setups,
    // creating a 200 Hz focus bounce — ipc.rs:424-428 documents this).
    const windowLabel =
        new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
    diag(`view-mount window_label=${windowLabel} initial-urlAtom=${JSON.stringify(model.urlAtom())}`);
    const [addressBar, setAddressBar] = createSignal(model.urlAtom() || "");
    let addressInputRef: HTMLInputElement | undefined;
    // Reactively mirror the model's URL into the address-bar input whenever
    // CEF reports a navigation via the `browser-pane-nav-state` event (in-
    // pane link clicks, redirects, back/forward, popup-intercept). Without
    // this the input stayed frozen at the last user-submitted text while
    // `model.urlAtom()` advanced, so the address bar diverged from the
    // actual pane URL. Skip while the user is actively editing the input
    // (focused) — otherwise we'd clobber mid-keystroke. Reagent caught
    // this on PR #484 review.
    createEffect(() => {
        const modelUrl = model.urlAtom();
        const focused = document.activeElement === addressInputRef;
        const willUpdate = !focused && modelUrl !== addressBar();
        diag(`sync urlAtom=${JSON.stringify(modelUrl)} addressBar=${JSON.stringify(addressBar())} focused=${focused} willUpdate=${willUpdate}`);
        if (focused) return;
        if (modelUrl !== addressBar()) setAddressBar(modelUrl);
    });
    let placeholderRef: HTMLDivElement | undefined;
    let resizeObserver: ResizeObserver | null = null;
    let positionInterval: ReturnType<typeof setInterval> | null = null;
    // SolidJS signal — must be reactive so <Show when={!paneCreated()}> re-runs
    // when the pane is created and hides the empty-state placeholder.
    const [paneCreated, setPaneCreated] = createSignal(false);

    // getBoundingClientRect() returns CSS pixels (device-INdependent); CEF
    // and Win32 SetWindowPos expect physical / device pixels. Multiply by
    // devicePixelRatio to convert. On HiDPI displays (dpr > 1) the pane
    // would be mispositioned/missized without this.
    const paneRect = () => {
        const r = placeholderRef!.getBoundingClientRect();
        const dpr = window.devicePixelRatio || 1;
        return {
            x: Math.round(r.x * dpr),
            y: Math.round(r.y * dpr),
            width: Math.round(r.width * dpr),
            height: Math.round(r.height * dpr),
        };
    };

    const syncPosition = () => {
        if (!placeholderRef || !paneCreated() || model.closed) return;
        invokeCommand("browser_pane_resize", {
            block_id: model.blockId,
            ...paneRect(),
        }).catch(() => {});
    };

    const createPane = async (url: string) => {
        if (!placeholderRef) return;
        try {
            // windowLabel hoisted at component scope — same value used by
            // both createPane and main_window_focus IPC dispatch.
            diag(`createPane url=${JSON.stringify(url)} window_label=${windowLabel}`);
            await invokeCommand("browser_pane_create", {
                block_id: model.blockId,
                url: url || "about:blank",
                window_label: windowLabel,
                ...paneRect(),
            });
            setPaneCreated(true);
            diag(`paneCreated=true`);
            model.onLoad();
        } catch (e) {
            model.onError(`Failed to create browser pane: ${e}`);
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

        model.navigate(normalized);
        setAddressBar(normalized);

        if (paneCreated()) {
            invokeCommand("browser_pane_navigate", {
                block_id: model.blockId,
                url: normalized,
            }).catch((e: any) => model.onError(`Navigation failed: ${e}`));
        } else {
            createPane(normalized);
        }
    };

    const handleAddressKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter") {
            e.preventDefault();
            handleNavigate();
        }
    };

    onMount(() => {
        if (placeholderRef) {
            resizeObserver = new ResizeObserver(syncPosition);
            resizeObserver.observe(placeholderRef);
            positionInterval = setInterval(syncPosition, 200);
        }
        const url = model.urlAtom();
        if (url) createPane(url);
    });

    onCleanup(() => {
        diag(`view-unmount paneCreated=${paneCreated()}`);
        // Fire close IPC BEFORE disconnecting observers — the IPC flips the
        // backend pane to Closing, so any in-flight resize/focus/nav calls
        // that haven't reached the backend yet get no-op'd there instead of
        // racing a mid-destruction HWND. See SPEC_BROWSER_PANE_LIFECYCLE.md §5.
        if (paneCreated()) {
            invokeCommand("browser_pane_close", { block_id: model.blockId }).catch(() => {});
        }
        resizeObserver?.disconnect();
        if (positionInterval) {
            clearInterval(positionInterval);
            positionInterval = null;
        }
    });

    return (
        <div class="browser-view">
            <div class="browser-nav-bar">
                <button
                    class="browser-nav-btn"
                    disabled={!model.canGoBackAtom()}
                    onClick={() => model.goBack()}
                    title="Back"
                >{"\u2190"}</button>
                <button
                    class="browser-nav-btn"
                    disabled={!model.canGoForwardAtom()}
                    onClick={() => model.goForward()}
                    title="Forward"
                >{"\u2192"}</button>
                <button
                    class="browser-nav-btn"
                    onClick={() => invokeCommand("browser_pane_reload", { block_id: model.blockId }).catch(() => {})}
                    title="Reload"
                >{"\u21BB"}</button>
                <input
                    ref={addressInputRef}
                    class="browser-address-bar"
                    type="text"
                    value={addressBar()}
                    onInput={(e) => setAddressBar(e.currentTarget.value)}
                    onKeyDown={handleAddressKeyDown}
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

            <Show when={model.errorAtom()}>
                <div class="browser-error">{model.errorAtom()}</div>
            </Show>

            <div
                class="browser-placeholder"
                ref={placeholderRef}
                onMouseDown={() => {
                    // User clicked into the pane — explicitly hand Windows-level
                    // keyboard focus to the pane HWND so subsequent keystrokes
                    // and mouse-wheel events route there.
                    //
                    // We used to grab focus on onMouseEnter (hover), but that
                    // created a loop: clicking the address bar released focus,
                    // then the cursor drifting back over the pane (inevitable —
                    // the address bar is right above it) re-grabbed focus
                    // before the user could type. Hover-focus is nicer for
                    // scroll-without-click, but it breaks keyboard routing so
                    // aggressively that the trade-off doesn't pay. Explicit
                    // click is the clear user intent.
                    if (paneCreated() && !model.closed) {
                        invokeCommand("browser_pane_focus", { block_id: model.blockId }).catch(() => {});
                    }
                }}
            >
                <Show when={!model.urlAtom() && !paneCreated()}>
                    <div class="browser-empty">
                        <div class="browser-empty-icon">{"\uD83C\uDF10"}</div>
                        <div class="browser-empty-text">Enter a URL above to browse</div>
                    </div>
                </Show>
            </div>
        </div>
    );
}
