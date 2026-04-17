// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, Show, type JSX } from "solid-js";
import type { BrowserViewModel } from "./browser-model";
import "./browser-view.scss";

export function BrowserViewComponent(props: ViewComponentProps<BrowserViewModel>): JSX.Element {
    const model = props.model;
    const [addressBar, setAddressBar] = createSignal(model.urlAtom() || "");

    const handleNavigate = () => {
        const url = addressBar().trim();
        if (url) model.navigate(url);
    };

    const handleAddressKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter") {
            e.preventDefault();
            handleNavigate();
        }
    };

    return (
        <div class="browser-view">
            {/* Navigation bar */}
            <div class="browser-nav-bar">
                <button
                    class="browser-nav-btn"
                    disabled={!model.canGoBackAtom()}
                    onClick={() => model.goBack()}
                    title="Back"
                >
                    {"\u2190"}
                </button>
                <button
                    class="browser-nav-btn"
                    disabled={!model.canGoForwardAtom()}
                    onClick={() => model.goForward()}
                    title="Forward"
                >
                    {"\u2192"}
                </button>
                <button
                    class="browser-nav-btn"
                    onClick={() => model.reload()}
                    title="Reload"
                >
                    {"\u21BB"}
                </button>
                <input
                    class="browser-address-bar"
                    type="text"
                    value={addressBar()}
                    onInput={(e) => setAddressBar(e.currentTarget.value)}
                    onKeyDown={handleAddressKeyDown}
                    onFocus={(e) => e.currentTarget.select()}
                    placeholder="Enter URL or search..."
                />
                <button class="browser-nav-btn browser-go-btn" onClick={handleNavigate}>
                    Go
                </button>
            </div>

            {/* Loading indicator */}
            <Show when={model.loadingAtom()}>
                <div class="browser-loading-bar" />
            </Show>

            {/* Error message */}
            <Show when={model.errorAtom()}>
                <div class="browser-error">
                    {model.errorAtom()}
                    <div class="browser-error-hint">
                        This site may block embedding. Try opening in an external browser.
                    </div>
                </div>
            </Show>

            {/* iframe content */}
            <Show when={model.urlAtom()}>
                <iframe
                    class="browser-iframe"
                    src={model.urlAtom()}
                    sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox"
                    referrerpolicy="no-referrer"
                    onLoad={() => {
                        model.onLoad();
                        // Update address bar to match current URL
                        setAddressBar(model.urlAtom());
                    }}
                    onError={() => model.onError("Failed to load page")}
                />
            </Show>

            {/* Empty state */}
            <Show when={!model.urlAtom()}>
                <div class="browser-empty">
                    <div class="browser-empty-icon">{"\uD83C\uDF10"}</div>
                    <div class="browser-empty-text">Enter a URL above to browse</div>
                </div>
            </Show>
        </div>
    );
}
