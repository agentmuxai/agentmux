// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { invokeCommand } from "@/app/platform/ipc";
import type { BrowserViewModel } from "./browser-model";
import "./browser-view.scss";

export function BrowserViewComponent(props: ViewComponentProps<BrowserViewModel>): JSX.Element {
    const model = props.model;
    const [addressBar, setAddressBar] = createSignal(model.urlAtom() || "");
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
        if (!placeholderRef || !paneCreated()) return;
        invokeCommand("browser_pane_resize", {
            block_id: model.blockId,
            ...paneRect(),
        }).catch(() => {});
    };

    const createPane = async (url: string) => {
        if (!placeholderRef) return;
        try {
            await invokeCommand("browser_pane_create", {
                block_id: model.blockId,
                url: url || "about:blank",
                ...paneRect(),
            });
            setPaneCreated(true);
            model.onLoad();
        } catch (e) {
            model.onError(`Failed to create browser pane: ${e}`);
        }
    };

    const handleNavigate = () => {
        const url = addressBar().trim();
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
        resizeObserver?.disconnect();
        if (positionInterval) clearInterval(positionInterval);
        if (paneCreated()) {
            invokeCommand("browser_pane_close", { block_id: model.blockId }).catch(() => {});
        }
    });

    return (
        <div class="browser-view">
            <div class="browser-nav-bar">
                <button
                    class="browser-nav-btn"
                    disabled={!model.canGoBackAtom()}
                    onClick={() => {
                        model.goBack();
                        invokeCommand("browser_pane_go_back", { block_id: model.blockId }).catch(() => {});
                    }}
                    title="Back"
                >{"\u2190"}</button>
                <button
                    class="browser-nav-btn"
                    disabled={!model.canGoForwardAtom()}
                    onClick={() => {
                        model.goForward();
                        invokeCommand("browser_pane_go_forward", { block_id: model.blockId }).catch(() => {});
                    }}
                    title="Forward"
                >{"\u2192"}</button>
                <button
                    class="browser-nav-btn"
                    onClick={() => invokeCommand("browser_pane_reload", { block_id: model.blockId }).catch(() => {})}
                    title="Reload"
                >{"\u21BB"}</button>
                <input
                    class="browser-address-bar"
                    type="text"
                    value={addressBar()}
                    onInput={(e) => setAddressBar(e.currentTarget.value)}
                    onKeyDown={handleAddressKeyDown}
                    onFocus={(e) => {
                        e.currentTarget.select();
                        // Take OS-level keyboard focus away from the pane so
                        // keystrokes reach this address-bar input. The pane may
                        // have held HWND focus from an earlier hover.
                        invokeCommand("main_window_focus", {}).catch(() => {});
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
                onMouseEnter={() => {
                    // Windows routes WM_MOUSEWHEEL to the focused HWND, so hand
                    // OS-level keyboard focus to the pane when the cursor is over
                    // it. Without this, wheel events go to main's widget and the
                    // embedded page can't scroll.
                    if (paneCreated()) {
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
