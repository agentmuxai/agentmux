// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Linux window controls, in the style of GNOME's title-bar buttons: round
 * buttons with a soft background behind 16 px symbolic glyphs, and a neutral
 * Close. They sit on the side and in the order of the desktop's own
 * button-layout setting, Maximize turns into Restore while the window is
 * maximized, and they dim while the window is not focused.
 * docs/specs/SPEC_LINUX_WINDOW_CONTROLS_GNOME_STYLE_2026_10_16.md
 *
 * The glyphs are drawn here, not taken from the Adwaita icon theme (which is
 * licensed separately).
 */

import { Button } from "@/app/element/ui/Button";
import { getApi } from "@/store/global";
import { useWindowDrag } from "@/app/hook/useWindowDrag.platform";
import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { windowButtonLayout, type WindowButton } from "./window-button-layout";
import "./window-controls.linux.scss";

/** How long a burst of resizes must go quiet before asking whether the window is maximized. */
export const MAXIMIZED_RESIZE_SETTLE_MS = 150;

// 16 px symbolic glyphs, 2 px strokes, the ink in the middle 8 px.
const MinimizeGlyph = (): JSX.Element => (
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
        <rect x="4" y="10" width="8" height="2" fill="currentColor" />
    </svg>
);

const MaximizeGlyph = (): JSX.Element => (
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
        <rect x="5" y="5" width="6" height="6" fill="none" stroke="currentColor" stroke-width="2" />
    </svg>
);

/** GNOME's restore is a smaller square, not two overlapping windows. */
const RestoreGlyph = (): JSX.Element => (
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
        <rect x="6" y="6" width="4" height="4" fill="none" stroke="currentColor" stroke-width="2" />
    </svg>
);

const CloseGlyph = (): JSX.Element => (
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
        <path d="M5 5L11 11M11 5L5 11" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
    </svg>
);

/** This window's host label: the URL's `windowLabel` for every window but the main one. */
function ownWindowLabel(): string {
    return new URLSearchParams(window.location.search).get("windowLabel") || "main";
}

/** The window's maximized state, from the host, re-asked after resizes settle. */
function useWindowMaximized(): () => boolean {
    const [maximized, setMaximized] = createSignal(false);
    const ask = () => {
        getApi()
            .windows.isMaximized?.(ownWindowLabel())
            // Unknown (null): keep showing what we had.
            ?.then((maximized) => {
                if (maximized != null) setMaximized(maximized);
            })
            .catch(() => {});
    };
    onMount(() => {
        ask();
        let timer: ReturnType<typeof setTimeout> | undefined;
        const onResize = () => {
            clearTimeout(timer);
            timer = setTimeout(ask, MAXIMIZED_RESIZE_SETTLE_MS);
        };
        window.addEventListener("resize", onResize);
        onCleanup(() => {
            clearTimeout(timer);
            window.removeEventListener("resize", onResize);
        });
    });
    return maximized;
}

/** Whether the window has focus: GNOME dims its buttons on an unfocused window. */
function useWindowFocused(): () => boolean {
    const [focused, setFocused] = createSignal(document.hasFocus());
    onMount(() => {
        const on = () => setFocused(true);
        const off = () => setFocused(false);
        window.addEventListener("focus", on);
        window.addEventListener("blur", off);
        onCleanup(() => {
            window.removeEventListener("focus", on);
            window.removeEventListener("blur", off);
        });
    });
    return focused;
}

function ControlButton(props: { kind: WindowButton; maximized: boolean }): JSX.Element {
    const spec = () => {
        switch (props.kind) {
            case "minimize":
                return { title: "Minimize", glyph: <MinimizeGlyph />, click: () => getApi().minimizeWindow() };
            case "maximize":
                return props.maximized
                    ? { title: "Restore", glyph: <RestoreGlyph />, click: () => getApi().maximizeWindow() }
                    : { title: "Maximize", glyph: <MaximizeGlyph />, click: () => getApi().maximizeWindow() };
            case "close":
                return {
                    title: "Close",
                    glyph: <CloseGlyph />,
                    click: () => void getApi().closeWindow().catch(console.error),
                };
        }
    };
    return (
        <Button
            tone="quiet"
            class={`window-action-btn ${props.kind}-btn`}
            onClick={() => spec().click()}
            title={spec().title}
            aria-label={spec().title}
            data-testid={`window-${props.kind}-btn`}
            data-drag-region="false"
        >
            <span class="window-action-glyph">{spec().glyph}</span>
        </Button>
    );
}

function GnomeWindowButtons(props: { side: "left" | "right" }): JSX.Element {
    const { dragProps } = useWindowDrag();
    const maximized = useWindowMaximized();
    const focused = useWindowFocused();
    const buttons = () => windowButtonLayout()[props.side];
    return (
        <Show when={buttons().length > 0}>
            <div
                class={`window-action-buttons gnome-window-buttons gnome-window-buttons--${props.side}`}
                classList={{ "window-unfocused": !focused() }}
                {...dragProps}
            >
                <For each={buttons()}>{(kind) => <ControlButton kind={kind} maximized={maximized()} />}</For>
            </div>
        </Show>
    );
}

export const WindowControlsLeft = (): JSX.Element => <GnomeWindowButtons side="left" />;
export const WindowControlsRight = (): JSX.Element => <GnomeWindowButtons side="right" />;
