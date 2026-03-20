// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { Workspace } from "@/app/workspace/workspace";
import { ContextMenuModel } from "@/store/contextmenu";
import { atoms, getApi, getSettingsPrefixAtom, isDev, openLink, removeFlashError, flashErrors } from "@/store/global";
import { appHandleKeyDown, keyboardMouseDownHandler } from "@/store/keymodel";
import { chromeZoomIn, chromeZoomOut, zoomBlockIn, zoomBlockOut, WHEEL_STEP } from "@/store/zoom.platform";
import { getElemAsStr } from "@/util/focusutil";
import * as keyutil from "@/util/keyutil";
import { PLATFORM } from "@/util/platformutil";
import * as util from "@/util/util";
import clsx from "clsx";
import debug from "debug";
import "overlayscrollbars/overlayscrollbars.css";
import { createEffect, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { AppBackground } from "./app-bg";
import { CrossWindowDragMonitor } from "./drag/CrossWindowDragMonitor";
import { DragOverlay } from "./drag/DragOverlay";
import { CenteredDiv } from "./element/quickelems";
import { ZoomIndicator } from "./element/zoomindicator";
import { NotificationBubbles } from "./notification/notificationbubbles";

import "./app.scss";

// tailwindsetup.css should come *after* app.scss (don't remove the newline above otherwise prettier will reorder these imports)
import "../tailwindsetup.css";

const dlog = debug("wave:app");
const focusLog = debug("wave:focus");

const App = () => {
    return <AppInner />;
};

function isContentEditableBeingEdited(): boolean {
    const activeElement = document.activeElement;
    return (
        activeElement &&
        activeElement.getAttribute("contenteditable") !== null &&
        activeElement.getAttribute("contenteditable") !== "false"
    );
}

function canEnablePaste(): boolean {
    const activeElement = document.activeElement;
    return activeElement.tagName === "INPUT" || activeElement.tagName === "TEXTAREA" || isContentEditableBeingEdited();
}

function canEnableCopy(): boolean {
    const sel = window.getSelection();
    return !util.isBlank(sel?.toString());
}

function canEnableCut(): boolean {
    const sel = window.getSelection();
    if (document.activeElement?.classList.contains("xterm-helper-textarea")) {
        return false;
    }
    return !util.isBlank(sel?.toString()) && canEnablePaste();
}

async function getClipboardURL(): Promise<URL> {
    try {
        const clipboardText = await navigator.clipboard.readText();
        if (clipboardText == null) {
            return null;
        }
        const url = new URL(clipboardText);
        if (!url.protocol.startsWith("http")) {
            return null;
        }
        return url;
    } catch (e) {
        return null;
    }
}

async function handleContextMenu(e: MouseEvent) {
    e.preventDefault();
    const canPaste = canEnablePaste();
    const canCopy = canEnableCopy();
    const canCut = canEnableCut();
    const clipboardURL = await getClipboardURL();
    if (!canPaste && !canCopy && !canCut && !clipboardURL) {
        return;
    }
    let menu: ContextMenuItem[] = [];
    if (canCut) {
        menu.push({ label: "Cut", role: "cut" });
    }
    if (canCopy) {
        menu.push({ label: "Copy", role: "copy" });
    }
    if (canPaste) {
        menu.push({ label: "Paste", role: "paste" });
    }
    if (clipboardURL) {
        menu.push({ type: "separator" });
        menu.push({
            label: "Open Clipboard URL (" + clipboardURL.hostname + ")",
            click: () => {
                openLink(clipboardURL.toString());
            },
        });
    }
    ContextMenuModel.showContextMenu(menu, e);
}

function AppSettingsUpdater() {
    const windowSettingsAtom = getSettingsPrefixAtom("window");
    createEffect(() => {
        const windowSettings = windowSettingsAtom();
        const isTransparentOrBlur =
            (windowSettings?.["window:transparent"] || windowSettings?.["window:blur"]) ?? false;
        const opacity = util.boundNumber(windowSettings?.["window:opacity"] ?? 0.8, 0, 1);
        const baseBgColor = windowSettings?.["window:bgcolor"];
        const mainDiv = document.getElementById("main");
        // console.log("window settings", windowSettings, isTransparentOrBlur, opacity, baseBgColor, mainDiv);
        if (isTransparentOrBlur) {
            mainDiv.classList.add("is-transparent");
            document.documentElement.style.background = "transparent";
            if (opacity != null) {
                document.body.style.setProperty("--window-opacity", `${opacity}`);
            } else {
                document.body.style.removeProperty("--window-opacity");
            }
        } else {
            mainDiv.classList.remove("is-transparent");
            document.documentElement.style.removeProperty("background");
            document.body.style.removeProperty("--window-opacity");
        }
        if (baseBgColor != null) {
            document.body.style.setProperty("--main-bg-color", baseBgColor);
        } else {
            document.body.style.removeProperty("--main-bg-color");
        }
        // Apply Tauri-level window transparency and platform blur effects
        const isBlur = windowSettings?.["window:blur"] ?? false;
        getApi().setWindowTransparency(isTransparentOrBlur, isBlur, opacity);
    });
    return null;
}

function appFocusIn(e: FocusEvent) {
    focusLog("focusin", getElemAsStr(e.target), "<=", getElemAsStr(e.relatedTarget));
}

function appFocusOut(e: FocusEvent) {
    focusLog("focusout", getElemAsStr(e.target), "=>", getElemAsStr(e.relatedTarget));
}

function appSelectionChange(e: Event) {
    const selection = document.getSelection();
    focusLog("selectionchange", getElemAsStr(selection.anchorNode));
}

function AppFocusHandler() {
    return null;

    // for debugging
    onMount(() => {
        document.addEventListener("focusin", appFocusIn);
        document.addEventListener("focusout", appFocusOut);
        document.addEventListener("selectionchange", appSelectionChange);
        const ivId = setInterval(() => {
            const activeElement = document.activeElement;
            if (activeElement instanceof HTMLElement) {
                focusLog("activeElement", getElemAsStr(activeElement));
            }
        }, 2000);
        onCleanup(() => {
            document.removeEventListener("focusin", appFocusIn);
            document.removeEventListener("focusout", appFocusOut);
            document.removeEventListener("selectionchange", appSelectionChange);
            clearInterval(ivId);
        });
    });
    return null;
}

const AppKeyHandlers = () => {
    onMount(() => {
        const staticKeyDownHandler = keyutil.keydownWrapper(appHandleKeyDown);
        document.addEventListener("keydown", staticKeyDownHandler);
        document.addEventListener("mousedown", keyboardMouseDownHandler);

        onCleanup(() => {
            document.removeEventListener("keydown", staticKeyDownHandler);
            document.removeEventListener("mousedown", keyboardMouseDownHandler);
        });
    });
    return null;
};

const AppZoomHandler = () => {
    onMount(() => {
        const handleWheel = (e: WheelEvent) => {
            // Only zoom if Ctrl/Cmd is held
            if (!e.ctrlKey && !e.metaKey) {
                return;
            }

            // Prevent default browser zoom
            e.preventDefault();

            const target = e.target as HTMLElement;
            const zoomOut = e.deltaY > 0;

            // Check if hovering over chrome (title bar, status bar, or pane header)
            if (target.closest(".window-header") || target.closest(".status-bar") || target.closest(".block-frame-default-header")) {
                if (zoomOut) chromeZoomOut(WHEEL_STEP);
                else chromeZoomIn(WHEEL_STEP);
                return;
            }

            // Otherwise zoom the terminal pane under the cursor
            const blockEl = target.closest("[data-blockid]");
            const blockId = blockEl?.getAttribute("data-blockid");
            if (!blockId) return;

            if (zoomOut) zoomBlockOut(blockId, WHEEL_STEP);
            else zoomBlockIn(blockId, WHEEL_STEP);
        };

        // Add with passive: false to allow preventDefault
        window.addEventListener("wheel", handleWheel, { passive: false });

        onCleanup(() => {
            window.removeEventListener("wheel", handleWheel);
        });
    });
    return null;
};

const FlashError = () => {
    const errors = flashErrors;
    const [hoveredId, setHoveredId] = createSignal<string>(null);
    const [ticker, setTicker] = createSignal<number>(0);

    createEffect(() => {
        const errs = errors();
        const hovered = hoveredId();
        // Track ticker to re-run on tick
        ticker();
        if (errs.length == 0 || hovered != null) {
            return;
        }
        const now = Date.now();
        for (let ferr of errs) {
            if (ferr.expiration == null || ferr.expiration < now) {
                removeFlashError(ferr.id);
            }
        }
        setTimeout(() => setTicker((t) => t + 1), 1000);
    });

    function copyError(id: string) {
        const errs = errors();
        const ferr = errs.find((f) => f.id === id);
        if (ferr == null) {
            return;
        }
        let text = "";
        if (ferr.title != null) {
            text += ferr.title;
        }
        if (ferr.message != null) {
            if (text.length > 0) {
                text += "\n";
            }
            text += ferr.message;
        }
        navigator.clipboard.writeText(text);
    }

    function convertNewlinesToBreaks(text: string) {
        return text.split("\n").map((part) => (
            <>
                {part}
                <br />
            </>
        ));
    }

    return (
        <Show when={errors().length > 0}>
            <div class="flash-error-container">
                <For each={errors()}>
                    {(err, idx) => (
                        <div
                            class={clsx("flash-error", { hovered: hoveredId() === err.id })}
                            onClick={() => copyError(err.id)}
                            onMouseEnter={() => setHoveredId(err.id)}
                            onMouseLeave={() => setHoveredId(null)}
                            title="Click to Copy Error Message"
                        >
                            <div class="flash-error-scroll">
                                <Show when={err.title != null}>
                                    <div class="flash-error-title">{err.title}</div>
                                </Show>
                                <Show when={err.message != null}>
                                    <div class="flash-error-message">{convertNewlinesToBreaks(err.message)}</div>
                                </Show>
                            </div>
                        </div>
                    )}
                </For>
            </div>
        </Show>
    );
};

const AppInner = () => {
    const prefersReducedMotion = atoms.prefersReducedMotionAtom;
    const client = atoms.client;
    const windowData = atoms.waveWindow;
    const isFullScreen = atoms.isFullScreen;

    return (
        <Show
            when={client() != null && windowData() != null}
            fallback={
                <div class="flex flex-col w-full h-full">
                    <AppBackground />
                    <CenteredDiv>invalid configuration, client or window was not loaded</CenteredDiv>
                </div>
            }
        >
            <div
                class={clsx("flex flex-col w-full h-full", PLATFORM, {
                    fullscreen: isFullScreen(),
                    "prefers-reduced-motion": prefersReducedMotion(),
                })}
                onContextMenu={handleContextMenu}
            >
                <AppBackground />
                <AppKeyHandlers />
                <AppZoomHandler />
                <AppFocusHandler />
                <AppSettingsUpdater />
                <Workspace />
                <CrossWindowDragMonitor />
                <DragOverlay />
                <FlashError />
                <Show when={isDev()}>
                    <NotificationBubbles />
                </Show>
                <ZoomIndicator />
            </div>
        </Show>
    );
};

export { App };
