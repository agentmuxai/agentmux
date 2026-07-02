// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { invokeCommand, listenEvent } from "@/app/platform/ipc";
import { FLOATER_EDGE_RESIZE_BORDER } from "@/app/workspace/floater-resize";
import { ModalLayer } from "@/element/ModalLayer";
import { useModalLayer } from "@/element/modal-layer";
import { registerPaneRect, unregisterPaneRect } from "@/app/platform/pane-rect-registry";
import { paneReflowActive, notifyPaneReflow } from "@/app/platform/pane-anim";
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

/**
 * Pane-scope modal host. Wraps the browser-pane content in a
 * `<ModalLayer scope="pane">` so any `useModalLayer()` call inside
 * resolves to THIS layer rather than the outer tab-scope one
 * (from `tabcontent.tsx`). The HTTP Basic / Digest auth modal that
 * fires on a 401-protected URL then locks only this pane —
 * everything else in the tab (sibling panes, tab bar, title bar)
 * stays interactive.
 *
 * Split into a thin outer + inner so the inner's `useModalLayer()`
 * call (line below) resolves against the wrapper's context, not the
 * caller's. Solid's hook resolves up the JSX tree at execution time,
 * so the consumer must be a CHILD of the provider — putting the
 * `useModalLayer()` call in the same function body as the
 * `<ModalLayer>` JSX would have it resolve to the outer (tab) layer
 * instead.
 * SPEC_LAUNCH_MODAL_PANE_SCOPE_2026_05_25.md §5 (browser-auth follow-up).
 */
export function BrowserViewComponent(props: ViewComponentProps<BrowserViewModel>): JSX.Element {
    return (
        <ModalLayer scope="pane">
            <BrowserViewInner model={props.model} />
        </ModalLayer>
    );
}

function BrowserViewInner(props: { model: BrowserViewModel }): JSX.Element {
    const model = props.model;
    const modalLayer = useModalLayer();
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
    // Last rect we actually sent to the host. syncPosition compares against
    // this and skips the IPC when nothing changed. Without this gate the
    // safety-net interval fired browser_pane_resize 5 ×/sec even when the
    // pane was steady, and each call ran controller.set_size +
    // set_position + window.layout() on the UI thread — visible as a 200ms
    // DOM blink as Views relayouted on every tick.
    let lastSentRect: { x: number; y: number; width: number; height: number } | null = null;
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
        let x = Math.round(r.x * dpr);
        const y = Math.round(r.y * dpr);
        let width = Math.round(r.width * dpr);
        let height = Math.round(r.height * dpr);
        // Floating browser pane: the floater's frontend DOM owns an invisible
        // edge grab band for edge-resize, but this pane's web-content child is
        // a separate OS window layered on top of it — so inset the child by the
        // band depth on the three window-edge sides (left/right/bottom; the top
        // edge is over the 33px header, already frontend) to expose the band.
        // The full-size placeholder div paints the strip (frontend, so no native
        // border). SPEC_FLOATING_PANE_EDGE_RESIZE.
        if (windowLabel.startsWith("floating-")) {
            const b = Math.round(FLOATER_EDGE_RESIZE_BORDER * dpr);
            x += b;
            width = Math.max(1, width - 2 * b);
            height = Math.max(1, height - b);
        }
        return { x, y, width, height };
    };

    /** CSS-pixel rect (same coordinate space as `getBoundingClientRect`
     *  on overlay elements). Stored in `pane-rect-registry` so
     *  `sendClip` can short-circuit when no overlay intersects a pane. */
    const paneRectCss = () => {
        const r = placeholderRef!.getBoundingClientRect();
        return {
            x: Math.round(r.x),
            y: Math.round(r.y),
            w: Math.round(r.width),
            h: Math.round(r.height),
        };
    };

    const syncPosition = () => {
        if (!placeholderRef || !paneCreated() || model.closed) return;
        const rect = paneRect();
        if (
            lastSentRect &&
            lastSentRect.x === rect.x &&
            lastSentRect.y === rect.y &&
            lastSentRect.width === rect.width &&
            lastSentRect.height === rect.height
        ) {
            return;
        }
        lastSentRect = rect;
        invokeCommand("browser_pane_resize", {
            block_id: model.blockId,
            ...rect,
        }).catch(() => {});
        // Keep the overlay-clip short-circuit registry in sync with the
        // host's actual HWND rect. Cheap (two property reads + a Map
        // write).
        registerPaneRect(model.blockId, paneRectCss());
    };

    // Native browser-pane HWND settle on a layout change.
    //
    // A native browser-pane HWND can't be moved by CSS. On a pane geometry
    // change `notifyPaneReflow()` opens a short window during which we re-sample
    // this pane's placeholder rect per frame and push it to the host
    // (`syncPosition` dedupes, so unchanged frames are free). The pane reflow CSS
    // animation has since been removed, so the placeholder no longer animates —
    // this now just settles the HWND onto the final rect; the ResizeObserver/poll
    // is the steady-state path.
    // See docs/specs/SPEC_PANE_REFLOW_ANIMATION_2026_05_29.md.
    let reflowRAF: number | null = null;
    const sampleReflowFrame = () => {
        syncPosition();
        if (paneReflowActive()) {
            reflowRAF = requestAnimationFrame(sampleReflowFrame);
        } else {
            // One final settle frame so the HWND lands exactly on the final
            // rect even if the last tick fired slightly early.
            reflowRAF = null;
            syncPosition();
        }
    };
    createEffect(() => {
        // `paneReflowActive()` reads the shared `animatingUntil` signal, so this
        // effect re-runs the instant a pane geometry change opens the settle
        // window; the per-frame loop then re-syncs the native browser-pane HWND
        // onto the new placeholder rect.
        if (paneReflowActive() && reflowRAF == null) {
            reflowRAF = requestAnimationFrame(sampleReflowFrame);
        }
    });

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
            // The HWND is now live — open a fresh settle window so the
            // per-frame loop positions it if the layout changed while the
            // async create was in-flight (reflow window may have expired).
            notifyPaneReflow();
            // Seed the overlay-clip short-circuit registry with the initial
            // rect; subsequent syncPosition ticks keep it current.
            registerPaneRect(model.blockId, paneRectCss());
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

    let authUnsub: (() => void) | null = null;
    let authDisposed = false;
    // Per-pane set of in-flight auth requests. ESC/backdrop dismiss
    // tears down the modal via `safeClose()` without firing the
    // browser-auth onCancel, so any remaining ids in this set need
    // explicit cancel on pane unmount AND on resolution. Tracks ids
    // so we cancel exactly the ones that didn't resolve through
    // the submit/cancel buttons.
    const pendingAuthIds = new Set<string>();
    // FIFO queue for concurrent auth challenges. Two protected
    // subresources on the same page (or two panes in the same tab)
    // can challenge before the user resolves the first prompt;
    // unconditional modalLayer.open would replace the visible modal,
    // and the unmounted panel's onCleanup would cancel the earlier
    // challenge — so authenticating the survivor still fails the
    // earlier requests. Queue new arrivals; open the next after the
    // active one resolves.
    type AuthChallenge = {
        request_id: string;
        origin: string;
        host: string;
        port: number;
        realm: string;
        is_proxy: boolean;
    };
    const authQueue: AuthChallenge[] = [];
    let authActive = false;

    const openAuthPrompt = (c: AuthChallenge) => {
        authActive = true;
        modalLayer.open({
            kind: "browser-auth",
            blockId: model.blockId,
            requestId: c.request_id,
            origin: c.origin || `${c.host}:${c.port}`,
            realm: c.realm,
            isProxy: c.is_proxy,
            onSubmit: (username, password) => {
                pendingAuthIds.delete(c.request_id);
                diag(`auth-submit request_id=${c.request_id}`);
                void invokeCommand("browser_pane_auth_submit", {
                    request_id: c.request_id,
                    username,
                    password,
                }).catch((e) => diag(`auth-submit-failed err=${String(e)}`));
                authActive = false;
                drainAuthQueue();
            },
            onCancel: () => {
                pendingAuthIds.delete(c.request_id);
                diag(`auth-cancel request_id=${c.request_id}`);
                void invokeCommand("browser_pane_auth_cancel", {
                    request_id: c.request_id,
                }).catch(() => {});
                authActive = false;
                drainAuthQueue();
            },
        });
    };
    const drainAuthQueue = () => {
        if (authActive || authQueue.length === 0) return;
        const next = authQueue.shift()!;
        // Defer to a microtask so the prior modal's onCleanup has
        // run before we mount the next one — replacing synchronously
        // would re-trigger the cleanup-cancel path the queue exists
        // to prevent.
        queueMicrotask(() => openAuthPrompt(next));
    };

    onMount(() => {
        if (placeholderRef) {
            resizeObserver = new ResizeObserver(syncPosition);
            resizeObserver.observe(placeholderRef);
            positionInterval = setInterval(syncPosition, 200);
        }
        // Subscribe to CEF's HTTP Basic/Digest auth challenges. Lives
        // in the view (not the model) because it needs `useModalLayer()`
        // context, which is a SolidJS hook. Phase α of
        // SPEC_BROWSER_PANE_HTTP_BASIC_AUTH_2026_05_18.md.
        void listenEvent<{
            block_id: string;
            request_id: string;
            origin: string;
            host: string;
            port: number;
            realm: string;
            is_proxy: boolean;
        }>("browser-pane-auth-required", (payload) => {
            if (payload.block_id !== model.blockId) return;
            // Diagnostic stays high-signal: request_id + origin + realm
            // are enough to trace the prompt without logging credentials.
            diag(`auth-required request_id=${payload.request_id} origin=${JSON.stringify(payload.origin)} realm=${JSON.stringify(payload.realm)}`);
            pendingAuthIds.add(payload.request_id);
            const challenge: AuthChallenge = {
                request_id: payload.request_id,
                origin: payload.origin,
                host: payload.host,
                port: payload.port,
                realm: payload.realm,
                is_proxy: payload.is_proxy,
            };
            if (authActive) {
                diag(`auth-queue request_id=${payload.request_id} depth=${authQueue.length + 1}`);
                authQueue.push(challenge);
            } else {
                openAuthPrompt(challenge);
            }
        }).then((unsub) => {
            // listenEvent's promise can resolve AFTER onCleanup has
            // already run (pane closed before subscription completed).
            // Without this check, the unsub closure is captured post-
            // cleanup and never invoked, leaking the listener until
            // renderer teardown.
            if (authDisposed) unsub();
            else authUnsub = unsub;
        });
        // macOS/Linux: after a JS-driven drag moves the floating pane window,
        // paneRect() returns the same client coords (viewport-relative, unchanged
        // by window movement), so syncPosition's dedupe guard skips the re-send.
        // SetPaneBoundsViewsTask computes the overlay's absolute screen position
        // from the CefNSWindow's CURRENT frame + client coords, so we must re-send
        // after drag to reposition the NativeWidgetMacNSWindow overlay.
        // floating-pane-workspace.tsx dispatches "floating-pane-js-drag-ended" on
        // window after every JS-driven drag so we can clear the dedupe guard here.
        if (windowLabel.startsWith("floating-")) {
            const onJsDragEnded = (ev: Event) => {
                const detail = (ev as CustomEvent<{ label: string }>).detail;
                if (detail?.label !== windowLabel) return;
                lastSentRect = null;
                syncPosition();
            };
            window.addEventListener("floating-pane-js-drag-ended", onJsDragEnded);
            onCleanup(() => window.removeEventListener("floating-pane-js-drag-ended", onJsDragEnded));
        }
        const url = model.urlAtom();
        if (url) createPane(url);
    });

    onCleanup(() => {
        diag(`view-unmount paneCreated=${paneCreated()}`);
        // Drop from the overlay-clip short-circuit registry FIRST so a
        // late sendClip() doesn't see a stale rect for the closed pane.
        unregisterPaneRect(model.blockId);
        // Fire close IPC BEFORE disconnecting observers — the IPC flips the
        // backend pane to Closing, so any in-flight resize/focus/nav calls
        // that haven't reached the backend yet get no-op'd there instead of
        // racing a mid-destruction HWND. See SPEC_BROWSER_PANE_LIFECYCLE.md §5.
        if (paneCreated()) {
            // Pass window_label so the host can ignore a stale close from a
            // window that no longer owns the pane. On tear-off/redock the pane
            // moves to another window and is recreated there; this old
            // component then unmounts and fires this close — without the label,
            // the host (which keys close on block_id) would destroy the moved
            // pane and black out the new window. See browser_pane_close in
            // ipc.rs + the AlreadyLiveElsewhere fix.
            invokeCommand("browser_pane_close", { block_id: model.blockId, window_label: windowLabel }).catch(() => {});
        }
        resizeObserver?.disconnect();
        if (positionInterval) {
            clearInterval(positionInterval);
            positionInterval = null;
        }
        if (reflowRAF != null) {
            cancelAnimationFrame(reflowRAF);
            reflowRAF = null;
        }
        authDisposed = true;
        if (authUnsub) {
            authUnsub();
            authUnsub = null;
        }
        // Cancel any auth prompts still parked on the host (active
        // + queued). The backend also fires `cancel_for_block` from
        // `browser_pane_close` as a safety net, but firing them here
        // ensures each cancel logs against the correct request_id.
        for (const requestId of pendingAuthIds) {
            invokeCommand("browser_pane_auth_cancel", { request_id: requestId })
                .catch(() => {});
        }
        pendingAuthIds.clear();
        authQueue.length = 0;
        authActive = false;
    });

    return (
        <div class="browser-view">
            <Show when={model.showControlsAtom()}>
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
            </Show>

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
