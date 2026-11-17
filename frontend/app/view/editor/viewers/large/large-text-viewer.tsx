// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's Large text viewer: a text file too big for the editor shown as
 * numbered lines, read a page at a time so a file of any size opens at once.
 * Go to a line, long lines cut short. Read-only.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.6.
 */

import { Button } from "@/app/element/ui";
import { tip } from "@/app/element/tip";
import { keyLabel } from "@/app/keybindings";
import { For, Show, type JSX } from "solid-js";
import { ViewerBar } from "../viewer-bar";
import { contentHeight, ROW_H } from "../hex/hex-layout";
import { createLargeText, type LargeTextProps, type LargeTextState } from "./large-text-state";
import "./large-text-viewer.scss";

export type { LargeTextProps } from "./large-text-state";

export function LargeTextViewer(props: LargeTextProps): JSX.Element {
    const t = createLargeText(props);
    return (
        <div class="editor-viewer large-text-viewer" style={{ "--large-line-h": `${ROW_H}px` }}>
            <ViewerBar {...props.bar} facts={t.facts()} start={<LargeTextTools t={t} />} />
            <Show when={t.error()}>
                <div class="large-text-error" role="alert">
                    <span>{t.error()}</span>
                    <Button density="compact" onClick={t.retry}>
                        Try again
                    </Button>
                </div>
            </Show>
            <div class="large-text-body">
                <Show when={t.total() !== 0} fallback={<div class="editor-viewer-message">This file is empty.</div>}>
                    <LineGrid t={t} />
                </Show>
            </div>
        </div>
    );
}

/** The viewer's own controls in the viewer bar. */
function LargeTextTools(props: { t: LargeTextState }): JSX.Element {
    const t = props.t;
    return (
        <input
            ref={t.bindLineBox}
            class="large-text-line-input"
            data-raw-text-field="the viewer bar's line box: a short number beside the other bar controls, with its own mod+g focus and Enter to go, not a form field"
            placeholder="Line"
            aria-label="Go to line"
            aria-invalid={t.badLine()}
            {...tip(`Go to a line (${keyLabel("mod+g")})`)}
            onInput={t.clearBadLine}
            onKeyDown={(e) => e.key === "Enter" && t.goToTyped(e.currentTarget.value)}
        />
    );
}

/** The lines on screen, placed in a spacer as tall as the file's lines. */
function LineGrid(props: { t: LargeTextState }): JSX.Element {
    const t = props.t;
    const digits = () => Math.max(3, String(t.lines()).length);
    return (
        <div
            ref={t.bind}
            class="large-text-grid"
            role="region"
            tabIndex={0}
            aria-label="The file's lines"
            style={{ "--large-digits": digits() }}
            onScroll={t.onScroll}
            onKeyDown={t.onKeyDown}
        >
            <div class="large-text-spacer" style={{ height: `${contentHeight(t.lines())}px` }}>
                <div class="large-text-lines" style={{ "margin-top": `${t.layerTop()}px` }}>
                    <For each={t.shown()}>{(n) => <Line t={t} n={n} />}</For>
                </div>
            </div>
        </div>
    );
}

function Line(props: { t: LargeTextState; n: number }): JSX.Element {
    const line = () => props.t.lineAt(props.n);
    return (
        <div class="large-text-line" classList={{ "large-text-marked": props.t.mark() === props.n + 1 }} data-line={props.n + 1}>
            <span class="large-text-number" aria-hidden="true">
                {props.n + 1}
            </span>
            <span class="large-text-text">
                {line()?.text}
                <Show when={line()?.hidden}>
                    <span class="large-text-hidden">…({line()?.hidden})</span>
                </Show>
            </span>
        </div>
    );
}
