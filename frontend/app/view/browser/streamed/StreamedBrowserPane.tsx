// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { usePaneTabVisibility } from "@/app/block/pane-tab-visibility";
import { t } from "@/app/i18n/i18n";
import { Button, IconButton, Notice, TextInput } from "@/app/element/ui";
import type { BrowserViewModel } from "../browser-model";
import { createFramePainter } from "./frame-painter";
import { attachStageInput } from "./stage-input";
import { addressToUrl, useStreamedPane, type PaneStatus, type StreamedPane } from "./use-streamed-pane";
import "./streamed-browser.scss";

/** What to say while the page is not live, in the person's language. */
function statusText(status: PaneStatus): string {
    switch (status) {
        case "waiting":
        case "connecting":
            return t("browser.streamed.connecting");
        case "failed":
            return t("browser.streamed.failed");
        case "ended":
            return t("browser.streamed.ended");
        case "paused":
            return t("browser.streamed.paused");
        default:
            return "";
    }
}

/** The bar over the page: back, forward, reload or stop, and the address. */
function StreamedBar(props: { pane: StreamedPane }): JSX.Element {
    const [typed, setTyped] = createSignal<string | null>(null);
    const nav = () => props.pane.nav();
    const shown = () => typed() ?? nav()?.url ?? "";
    const go = () => {
        const url = addressToUrl(typed() ?? "");
        setTyped(null);
        if (url) props.pane.send({ t: "nav", go: "url", url });
    };
    return (
        <div class="streamed-bar">
            <IconButton icon="arrow-left" label={t("browser.streamed.back")} disabled={!nav()?.canGoBack} onClick={() => props.pane.send({ t: "nav", go: "back" })} />
            <IconButton icon="arrow-right" label={t("browser.streamed.forward")} disabled={!nav()?.canGoForward} onClick={() => props.pane.send({ t: "nav", go: "forward" })} />
            <Show
                when={nav()?.loading}
                fallback={<IconButton icon="rotate-right" label={t("browser.streamed.reload")} onClick={() => props.pane.send({ t: "nav", go: "reload" })} />}
            >
                <IconButton icon="stop" label={t("browser.streamed.stop")} onClick={() => props.pane.send({ t: "nav", go: "stop" })} />
            </Show>
            <TextInput
                class="streamed-bar-address"
                spellcheck={false}
                aria-label={t("browser.streamed.address")}
                value={shown()}
                onInput={(e) => setTyped(e.currentTarget.value)}
                onFocus={(e) => e.currentTarget.select()}
                onBlur={() => setTyped(null)}
                onKeyDown={(e) => {
                    if (e.key === "Enter") go();
                    if (e.key === "Escape") setTyped(null);
                }}
            />
        </div>
    );
}

/**
 * A Browser pane for a host that has no native browser: the page runs in a Chromium on the server and arrives as
 * frames on a canvas; the mouse, wheel, keyboard and paste go back. The socket is opened when the pane is on screen
 * and has a size, and closed when it is hidden or goes.
 * docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md §7.
 */
export function StreamedBrowserPane(props: { model: BrowserViewModel }): JSX.Element {
    let stage!: HTMLDivElement;
    let canvas: HTMLCanvasElement | undefined;
    const painter = createFramePainter(() => canvas);
    const pane = useStreamedPane(
        painter.draw,
        () => (props.model.meta()?.["url"] as string | undefined) ?? "",
        (url) => props.model.rememberPageUrl(url)
    );

    // On screen means in view and in a tab the person can see: a kept-alive hidden tab keeps its box in the viewport.
    const [inView, setInView] = createSignal(false);
    const tabVisibility = usePaneTabVisibility(props.model.blockId);
    createEffect(() => pane.shown(inView() && tabVisibility() === "active"));

    onMount(() => {
        const sizes = new ResizeObserver(() => pane.resized(stage.clientWidth, stage.clientHeight));
        const sight = new IntersectionObserver((entries) => setInView(entries.some((e) => e.isIntersecting)));
        sizes.observe(stage);
        sight.observe(stage);
        pane.resized(stage.clientWidth, stage.clientHeight);
        const unwire = attachStageInput(stage, () => canvas, pane.send);
        onCleanup(() => {
            sizes.disconnect();
            sight.disconnect();
            unwire();
        });
    });

    const message = () => statusText(pane.status());

    return (
        <div class="browser-view streamed-browser">
            <StreamedBar pane={pane} />
            <div class="streamed-stage" ref={stage} tabindex={0}>
                <canvas class="streamed-canvas" ref={canvas} />
                <Show when={message()}>
                    <div class="streamed-status" role="status">
                        <span>{message()}</span>
                        <Show when={pane.status() === "failed" || pane.status() === "ended"}>
                            <Button onClick={pane.reconnect}>{t("browser.streamed.reconnect")}</Button>
                        </Show>
                    </div>
                </Show>
                {/* After the status, so a terminal error stays readable. Over the page, not above it: showing or dismissing it must not resize the page. */}
                <Show when={pane.error()}>
                    <div class="streamed-notice">
                        <Notice tone="warning" icon="triangle-exclamation" onDismiss={pane.dismissError}>
                            {pane.error()}
                        </Notice>
                    </div>
                </Show>
            </div>
        </div>
    );
}
