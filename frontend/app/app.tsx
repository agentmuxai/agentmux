// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Install the replaceChild crash diagnostic FIRST, before any rendering module
// loads, so it's active when the agent pane mounts. See the module header,
// SPEC_REPLACECHILD_CRASH_FULL_ANALYSIS_AND_FIX_2026-06-06.md §7.4, and #1326.
import "./diag/replace-child-diagnostic";

import { Workspace } from "@/app/workspace/workspace";
import { FloatingPaneWorkspace } from "@/app/workspace/floating-pane-workspace";
import { showTextInputContextMenu } from "@/store/contextmenu";
import { LIGHT_THEME_IDS } from "@/app/menu/base-menus";
import { atoms, getApi, getSettingsPrefixAtom, isDev, removeFlashError, flashErrors } from "@/store/global";
import { appHandleKeyDown, keyboardMouseDownHandler } from "@/store/keymodel";
import { chromeZoomIn, chromeZoomOut, zoomBlockIn, zoomBlockOut, WHEEL_STEP } from "@/store/zoom.platform";
import { getElemAsStr } from "@/util/focusutil";
import * as keyutil from "@/util/keyutil";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { PLATFORM } from "@/util/platformutil";
import * as util from "@/util/util";
import clsx from "clsx";
import debug from "debug";
import "overlayscrollbars/overlayscrollbars.css";
import { createEffect, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { AppBackground } from "./app-bg";
import { CrossWindowDragMonitor } from "./drag/CrossWindowDragMonitor.platform";
import { DragOverlay } from "./drag/DragOverlay";
import { CenteredDiv } from "./element/quickelems";
import { ZoomIndicator } from "./element/zoomindicator";
import { PerfHud } from "@/perf/hud";
import { DiagPanel } from "./devtools/diag-panel";
import { checkSeparatorParity, setupDprTracking } from "./init/dpr";
import { NotificationBubbles } from "./notification/notificationbubbles";
import { MemoryPressureBanner } from "./notification/memory-pressure-banner";
import { BrowserPaneOutsideClickBridge } from "./window/browser-pane-outside-click-bridge";

import "./app.scss";

setupDprTracking();
// Vite-evaluated build-time flag — no API/IPC dependency, safe at module load.
if (import.meta.env.DEV) checkSeparatorParity();

// tailwindsetup.css should come *after* app.scss (don't remove the newline above otherwise prettier will reorder these imports)
import "../tailwindsetup.css";

const dlog = debug("wave:app");
const focusLog = debug("wave:focus");

const App = () => {
    return <AppInner />;
};

function AppSettingsUpdater() {
    const windowSettingsAtom = getSettingsPrefixAtom("window");
    createEffect(() => {
        const windowSettings = windowSettingsAtom();
        const isTransparentOrBlur =
            (windowSettings?.["window:transparent"] || windowSettings?.["window:blur"]) ?? false;
        const opacity = util.boundNumber(windowSettings?.["window:opacity"] ?? 0.8, 0, 1);
        const baseBgColor = windowSettings?.["window:bgcolor"];
        // `#main` may not exist on the first effect run: AppSettingsUpdater is
        // a sibling of <Workspace /> (which owns the main div), and SolidJS's
        // mount order is sibling-by-sibling. Without a null guard, this
        // effect threw on the first run and SolidJS lost the subscription —
        // settings.json could say `window:transparent: true` and the effect
        // would never re-fire on later settings ticks. Optional chaining
        // means the effect runs cleanly on the first (mainDiv-less) tick,
        // subscribes to windowSettingsAtom, and gets re-fired once both
        // settings AND the main div are ready.
        const mainDiv = document.getElementById("main");
        // `--window-opacity` MUST be set on :root, not body. theme.scss declares
        // `--main-bg-color: rgba(34, 34, 34, var(--window-opacity, 1))` on
        // :root. CSS substitutes var() at the element where the custom
        // property is computed — so at :root, with :root's --window-opacity.
        // Descendants inherit the already-substituted value. Setting
        // --window-opacity only on body leaves :root's --main-bg-color at
        // alpha=1, and all 30+ panes that use `background: var(--main-bg-color)`
        // stay fully opaque even with window:transparent on.
        if (isTransparentOrBlur) {
            mainDiv?.classList.add("is-transparent");
            document.documentElement.style.background = "transparent";
            if (opacity != null) {
                document.documentElement.style.setProperty("--window-opacity", `${opacity}`);
            } else {
                document.documentElement.style.removeProperty("--window-opacity");
            }
        } else {
            mainDiv?.classList.remove("is-transparent");
            document.documentElement.style.removeProperty("background");
            // Explicitly set opacity=1 to override theme.scss's translucent
            // default (0.45). The default is chosen for first-paint
            // alpha-awareness in the common transparent-window case;
            // non-transparent windows need to flip it back to fully opaque.
            document.documentElement.style.setProperty("--window-opacity", "1");
        }
        if (baseBgColor != null) {
            document.body.style.setProperty("--main-bg-color", baseBgColor);
        } else {
            document.body.style.removeProperty("--main-bg-color");
        }
        // Apply Tauri-level window transparency and platform blur effects.
        // The IPC call can throw early in startup if window.api hasn't been
        // wired yet (visible as "[getApi] called before window.api exists" in
        // the console). Without try/catch the throw aborts this effect and
        // SolidJS never re-runs it on the next settings tick — leaving the
        // CSS classes set above untouched, and the body opaque. The CEF host
        // also reads CefSettings.background_color directly at init time, so
        // the IPC is only the "Tauri-side window.transparent" mirror; we can
        // safely skip it when the API isn't ready.
        const isBlur = windowSettings?.["window:blur"] ?? false;
        try {
            getApi().setWindowTransparency(isTransparentOrBlur, isBlur, opacity);
        } catch (e) {
            // Swallow — the CSS path above is what actually drives visual
            // transparency on Linux/Wayland under CEF. The Tauri-era IPC
            // mirror is best-effort.
        }

        // Apply color theme
        const theme = windowSettings?.["window:theme"];
        if (theme && theme !== "default") {
            document.documentElement.setAttribute("data-theme", theme);
        } else {
            document.documentElement.removeAttribute("data-theme");
        }
        // Generic light/dark polarity marker, independent of which specific
        // theme is active — see LIGHT_THEME_IDS for why this exists.
        if (theme && LIGHT_THEME_IDS.has(theme)) {
            document.documentElement.setAttribute("data-theme-polarity", "light");
        } else {
            document.documentElement.removeAttribute("data-theme-polarity");
        }
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
        clipboardWriteText(text);
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
    // Evaluated at component-mount time (not module-load time) so the pane
    // pool fast path works: awaitPanePoolPromote() calls replaceState() to
    // inject floatingPaneId into the URL BEFORE initHostNewWindow() calls
    // render(App) — but a module-level IIFE fires before any of that, so it
    // would always see the original ?pane-pool=1 URL and return false.
    // Cold path (open_floating_pane_window) opens the window with floatingPaneId
    // in the URL from the start, so both paths are correct here.
    const IS_FLOATING_PANE = new URLSearchParams(window.location.search).has("floatingPaneId");
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
                    "floating-pane-mode": IS_FLOATING_PANE,
                })}
                onContextMenu={showTextInputContextMenu}
            >
                <AppBackground />
                <AppKeyHandlers />
                <AppZoomHandler />
                <AppFocusHandler />
                <AppSettingsUpdater />
                <BrowserPaneOutsideClickBridge />
                <Show when={!IS_FLOATING_PANE}>
                    {/* Low-memory warning banners — app-wide, non-modal,
                        dismissible. Driven by the host's mem_pressure level.
                        RAM and Page File are independently-tracked signals
                        (SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16 §5.F,
                        SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07) — both
                        can show at once if both are true. */}
                    <MemoryPressureBanner kind="ram" />
                    <MemoryPressureBanner kind="pagefile" />
                </Show>
                <Show
                    when={IS_FLOATING_PANE}
                    fallback={<Workspace />}
                >
                    <FloatingPaneWorkspace />
                </Show>
                <CrossWindowDragMonitor />
                <DragOverlay />
                <FlashError />
                <Show when={isDev()}>
                    <NotificationBubbles />
                </Show>
                <ZoomIndicator />
                <Show when={isDev()}>
                    <PerfHud />
                    <DiagPanel />
                </Show>
            </div>
        </Show>
    );
};

export { App };
