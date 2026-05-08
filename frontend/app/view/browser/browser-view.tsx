// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { invokeCommand } from "@/app/platform/ipc";
import type { BrowserViewModel } from "./browser-model";
import "./browser-view.scss";

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
                    onFocus={(e) => {
                        const tag = (el: Element | null) => {
                            if (!el) return "null";
                            const t = el.tagName?.toLowerCase() ?? "?";
                            const cls = (el as HTMLElement).className?.toString().split(/\s+/).find((c) => c) ?? "";
                            const id = (el as HTMLElement).id ?? "";
                            return `${t}${id ? `#${id}` : ""}${cls ? `.${cls}` : ""}`;
                        };
                        const wasAlreadyFocused = document.activeElement === e.currentTarget;
                        const realTransition = !!(e as any).relatedTarget;
                        diag(`input-focus value=${JSON.stringify(addressBar())} prev-active=${tag(document.activeElement)} same-target=${e.currentTarget === document.activeElement} relatedTarget=${tag((e as any).relatedTarget ?? null)} wasAlreadyFocused=${wasAlreadyFocused} realTransition=${realTransition}`);
                        e.currentTarget.select();
                        // Skip the main_window_focus IPC on synthetic re-fires.
                        // Chromium dispatches a synthetic focus/blur cycle on
                        // the DOM-focused input whenever a Win32 HWND focus
                        // shift happens upstream — we observed the input
                        // staying focused (now-active=this same input) across
                        // the whole bounce, with relatedTarget=null. Real
                        // user-initiated focus has relatedTarget set OR
                        // arrives at an element that wasn't already focused.
                        // Without this guard, every IPC triggers another
                        // synthetic event, which triggers another IPC — a
                        // ~200 Hz loop in multi-window setups (the IPC
                        // misroutes to the wrong window without window_label,
                        // see ipc.rs:424-428).
                        if (wasAlreadyFocused && !realTransition) {
                            diag(`input-focus skip-ipc reason=synthetic-refire`);
                            return;
                        }
                        invokeCommand("main_window_focus", { window_label: windowLabel }).catch(() => {});
                    }}
                    onBlur={(e) => {
                        const tag = (el: Element | null) => {
                            if (!el) return "null";
                            const t = el.tagName?.toLowerCase() ?? "?";
                            const cls = (el as HTMLElement).className?.toString().split(/\s+/).find((c) => c) ?? "";
                            return `${t}${cls ? `.${cls}` : ""}`;
                        };
                        // Microtask-deferred so we can see what landed focus AFTER the blur.
                        const next = (e as any).relatedTarget ?? null;
                        queueMicrotask(() => {
                            diag(`input-blur value=${JSON.stringify(addressBar())} relatedTarget=${tag(next)} now-active=${tag(document.activeElement)}`);
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
