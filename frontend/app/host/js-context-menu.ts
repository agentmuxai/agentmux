// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The context menu as a positioned HTML overlay: what `AppApi.showContextMenu`
// shows for a host with no native menu of its own. Plain DOM, no host calls,
// so every host uses this one (`jsContextMenuApi`). Moved out of the CEF shim
// (util/cef-api.ts), which was its only user.

import { registerPaneOverlay, type PaneOverlayHandle } from "@/app/platform/pane-overlay";
import {
    assertMenuInPaintableArea,
    computeMenuPosition,
    type MenuPositionResult,
} from "@/app/util/menu-position";
import { createSubmenuHover, type SubmenuHoverController } from "@/app/util/submenu-hover";

// Context menu click callback — registered by onContextMenuClick, called by showJsContextMenu.
let contextMenuClickCallback: ((id: string) => void) | null = null;

// Close function of the currently open showJsContextMenu, if any.
let activeContextMenuClose: (() => void) | null = null;

/** Close the open JS context menu (if any), releasing its pane-overlay holes. */
export function closeJsContextMenu(): void {
    activeContextMenuClose?.();
}

/**
 * Apply a computed MenuPositionResult to a menu element: fixed left/top plus
 * the size() max-height cap so a menu taller than the free space scrolls
 * internally instead of being placed partly outside. max-width is deliberately
 * NOT applied — an inline max-width would override (and can loosen) the .menu
 * 400px CSS cap, and horizontal fit is already guaranteed by flip+shift for
 * menus at or under that cap. justify-content is forced to flex-start because
 * the .menu rule's flex-end makes overflow unreachable in a scroll container
 * (it is a no-op when the menu fits, so this only matters when capped).
 */
function applyMenuPosition(el: HTMLElement, pos: MenuPositionResult) {
    el.style.position = "fixed";
    if (pos.style.left != null) el.style.left = String(pos.style.left);
    if (pos.style.top != null) el.style.top = String(pos.style.top);
    el.style.maxHeight = `${pos.maxHeight}px`;
    el.style.overflowY = "auto";
    el.style.justifyContent = "flex-start";
}

/**
 * Render a context menu as a positioned HTML overlay.
 * Fires the callback with the clicked item's id, then removes the overlay.
 * Exported for tests only — production callers go through showContextMenu.
 */
export function showJsContextMenu(
    items: NativeContextMenuItem[],
    position: { x: number; y: number },
    onClick: ((id: string) => void) | null
) {
    // Remove any existing menu (through its own close path, so its pane
    // overlay holes are released too).
    closeJsContextMenu();
    document.getElementById("cef-context-menu-overlay")?.remove();

    // Where the caret was when the menu opened (a terminal's textarea, an
    // input), so a menu that closes without moving focus elsewhere hands it
    // back. SPEC_CONTEXT_MENU_PASTE_KEEPS_TERMINAL_FOCUS_2026_10_07.md.
    const opener = document.activeElement instanceof HTMLElement && document.activeElement !== document.body
        ? document.activeElement
        : null;

    const overlay = document.createElement("div");
    overlay.id = "cef-context-menu-overlay";
    Object.assign(overlay.style, {
        position: "fixed", inset: "0", zIndex: "99999",
    });

    // Every close path goes through closeMenu(): it releases the pane-overlay
    // registrations (the menu and each submenu punch a hole through any
    // browser pane they overlap) and the document listeners before removing
    // the DOM. A leaked registration would leave a see-through,
    // click-through hole in the pane after the menu is gone.
    const overlayHandles: PaneOverlayHandle[] = [];
    const onDocMouseDown = (e: MouseEvent) => {
        // The backdrop covers the whole window, so a real click lands on it.
        // What reaches here from outside is the synthetic body mousedown that
        // browser-pane-outside-click-bridge.ts fires for a click inside a
        // native browser pane — treat it as an outside click.
        if (!(e.target instanceof Node) || !overlay.contains(e.target)) closeMenu();
    };
    const onDocKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            closeMenu();
        }
    };
    function closeMenu() {
        if (activeContextMenuClose === closeMenu) activeContextMenuClose = null;
        for (const h of overlayHandles) h.release();
        overlayHandles.length = 0;
        document.removeEventListener("mousedown", onDocMouseDown, true);
        document.removeEventListener("keydown", onDocKeyDown, true);
        overlay.remove();
        // After the chosen item has run (it runs right after closeMenu): if
        // focus fell to <body> or went with the removed menu, give it back to
        // the opener. An item that moved focus on purpose (a split's new pane,
        // Inspect) keeps it.
        queueMicrotask(() => {
            const active = document.activeElement;
            const lost = active == null || active === document.body || overlay.contains(active);
            if (lost && opener?.isConnected) opener.focus({ preventScroll: true });
        });
    }
    activeContextMenuClose = closeMenu;
    document.addEventListener("mousedown", onDocMouseDown, true);
    document.addEventListener("keydown", onDocKeyDown, true);

    overlay.addEventListener("mousedown", (e) => {
        // A press on the menu or its backdrop must not take focus: the browser
        // otherwise blurs the focused element (a terminal's textarea) to
        // <body>, and after "Paste" the user's next Enter went nowhere.
        // The click still fires; only the focus change is cancelled.
        e.preventDefault();
        if (e.target === overlay) closeMenu();
    });

    const menuEl = document.createElement("div");
    menuEl.className = "menu";
    menuEl.setAttribute("data-pane-overlay", "");
    menuEl.style.left = `${position.x}px`;
    menuEl.style.top = `${position.y}px`;
    // Hidden until computeMenuPosition places it — avoids a one-frame flash
    // at the cursor and stops the pane-overlay clip from firing for a
    // not-yet-positioned menu (visibility:hidden de-registers the rect).
    menuEl.style.visibility = "hidden";

    function renderItems(container: HTMLElement, itemList: NativeContextMenuItem[]) {
        // Peers (same-level submenu-bearing rows) close each other instantly
        // on entry — matches the old zero-delay mouseleave's implicit
        // sibling-closing side effect, which createSubmenuHover's open-delay/
        // safe-triangle close otherwise silently drops (reagent P1 on
        // PR #2525; the terminal's Themes/Font Size/Terminal Zoom/
        // Transparency submenus were the concrete case). One list
        // per renderItems call — each call is exactly one menu level, so
        // nested submenus never reach across levels to close an ancestor.
        const peers: SubmenuHoverController[] = [];
        for (const item of itemList) {
            if (item.type === "separator") {
                const sep = document.createElement("div");
                sep.className = "menu-divider";
                container.appendChild(sep);
                continue;
            }
            if (item.visible === false) continue;

            const row = document.createElement("div");
            row.className = "menu-item";
            if (item.enabled === false) {
                row.style.cursor = "default";
                row.style.opacity = "0.4";
                row.style.pointerEvents = "none";
            }

            // Radio/checkbox indicator slot — same FA-icon shape the
            // FlyoutMenu uses. When checked, render fa-check in accent
            // color; when unchecked but in a check-capable group,
            // render a blank-width spacer so labels stay aligned.
            const isCheckableType = item.type === "radio" || item.type === "checkbox";
            if (isCheckableType || item.checked !== undefined) {
                const icon = document.createElement("i");
                icon.className = item.checked
                    ? "fa-solid fa-fw fa-check menu-item-icon menu-item-check"
                    : "fa-solid fa-fw menu-item-icon menu-item-check";
                row.appendChild(icon);
            }

            // Inline color swatch (exact hue) rendered before the label — used
            // by the pane "Pane Color" submenu. This is a real DOM square, not an
            // emoji, so it matches our palette colors precisely.
            if (item.swatchColor) {
                const swatch = document.createElement("span");
                swatch.className = "menu-item-swatch";
                swatch.style.backgroundColor = item.swatchColor;
                row.appendChild(swatch);
            }

            const label = document.createElement("span");
            label.className = "label";
            label.textContent = item.label ?? "";
            row.appendChild(label);

            // Secondary, muted text after the label (right-aligned by the label's
            // flex-grow) — e.g. the bind-to-agent menu's live binding overview,
            // or the reason a row is disabled. ContextMenuItem.sublabel was
            // accepted and forwarded here but never drawn.
            if (item.sublabel) {
                const sub = document.createElement("span");
                sub.className = "menu-item-sublabel";
                sub.textContent = item.sublabel;
                row.appendChild(sub);
            }

            if (item.submenu && item.submenu.length > 0) {
                // Static CSS fallback (the pre-framework behavior): anchor at
                // the row's right edge, which needs the row as positioned
                // ancestor. Kept so a computeMenuPosition rejection still
                // yields a positioned submenu; the framework placement below
                // overrides it with fixed viewport coords on success.
                row.style.position = "relative";

                const arrow = document.createElement("i");
                arrow.className = "fa-sharp fa-solid fa-chevron-right";
                row.appendChild(arrow);

                const sub = document.createElement("div");
                sub.className = "menu sub-menu";
                sub.setAttribute("data-pane-overlay", "");
                sub.style.display = "none";
                sub.style.left = "100%";
                sub.style.top = "0";
                renderItems(sub, item.submenu);
                row.appendChild(sub);
                // display:none until hovered → no hole until it's shown and
                // placed; the style observer picks up each open/close.
                overlayHandles.push(registerPaneOverlay(sub));
                // Positioned through the shared framework, like the top-level
                // menu: anchored to the row, preferring right-start — flip()
                // sends it left of the parent near the right edge, shift()
                // pulls it up near the bottom, size() caps it when taller than
                // the free space. The computed coords are position:fixed, so
                // staying nested inside the row (which the hover logic needs)
                // doesn't affect placement. Held visibility:hidden until
                // placed for the same reason as the parent menu: the
                // pane-overlay clip must register the final rect only.
                //
                // Open/close timing goes through the shared hover-intent core
                // (SPEC_SUBMENU_POSITIONING_AND_HOVER_TIMING_2026_08_10) instead
                // of firing instantly on mouseenter/mouseleave: a short open
                // delay avoids flashing a submenu while the cursor is just
                // sweeping across sibling rows, and a safe-triangle close lets
                // the user travel diagonally into the submenu without it
                // vanishing out from under the cursor.
                const hover = createSubmenuHover({
                    onOpen: () => {
                        sub.style.visibility = "hidden";
                        sub.style.display = "";
                        // Deferred one rAF (mirrors flyoutmenu.tsx's SubMenu /
                        // registerSubMenu) so the display:none→"" reflow has
                        // settled before anything gets measured.
                        requestAnimationFrame(() => {
                            if (!sub.isConnected || sub.style.display === "none") return;
                            // The actual bug (root-caused live 2026-08-13,
                            // reproduced deterministically regardless of anchor
                            // position — left:121px, top:-393px every time,
                            // ruling out a mere layout-timing race): `sub`
                            // starts `position:absolute` (inherited from the
                            // `.menu` class) as a deliberate row-relative
                            // fallback for the .catch() below, but
                            // computeMenuPosition computes coordinates for
                            // `strategy:"fixed"`. floating-ui resolves the
                            // floating element's offset parent from its
                            // CURRENT position at measurement time, so calling
                            // it while `sub` is still `position:absolute`
                            // (nested inside `row`, which has
                            // `position:relative`) resolves coordinates
                            // against the wrong containing block entirely.
                            // FlyoutMenu's Solid sibling never hits this
                            // because its placeholder style is already
                            // `position:fixed;left:0px;top:0px` before it ever
                            // calls computeMenuPosition — match that here:
                            // switch to fixed strategy BEFORE measuring, and
                            // restore the absolute row-relative fallback
                            // explicitly if placement itself fails.
                            sub.style.position = "fixed";
                            void computeMenuPosition(
                                {
                                    anchor: row.getBoundingClientRect(),
                                    placement: "right-start",
                                    avoidNativePanes: false,
                                },
                                sub,
                            ).then((pos) => {
                                // Hover may have left (or the whole menu closed)
                                // before the async placement resolved.
                                if (!sub.isConnected || sub.style.display === "none") return;
                                applyMenuPosition(sub, pos);
                                sub.style.visibility = "";
                                assertMenuInPaintableArea(sub, "context-submenu");
                            }).catch(() => {
                                if (!sub.isConnected) return;
                                // Restore the row-relative absolute fallback —
                                // position:fixed with left:100% would otherwise
                                // pin it just off the right edge of the screen.
                                sub.style.position = "absolute";
                                sub.style.left = "100%";
                                sub.style.top = "0";
                                sub.style.visibility = "";
                            });
                        });
                    },
                    onClose: () => {
                        sub.style.display = "none";
                    },
                });
                // `sub` reports a zero rect via getBoundingClientRect() while
                // display:none, which the controller treats as "no geometry
                // yet" — safe to register once, up front.
                hover.setSubmenuEl(sub);
                peers.push(hover);
                row.addEventListener("mouseenter", () => {
                    for (const peer of peers) {
                        if (peer !== hover) peer.close();
                    }
                    hover.onTriggerEnter();
                });
                row.addEventListener("mouseleave", (e) => hover.onTriggerLeave(e as MouseEvent));
                sub.addEventListener("mouseenter", () => hover.onSubmenuEnter());
                sub.addEventListener("mouseleave", (e) => hover.onSubmenuLeave(e as MouseEvent));
            } else if (item.enabled !== false) {
                // A plain (no-submenu) row is still an explicit new selection —
                // entering it closes any open peer submenu immediately too.
                row.addEventListener("mouseenter", () => {
                    for (const peer of peers) peer.close();
                });
                row.addEventListener("click", () => {
                    closeMenu();
                    if (item.id && onClick) onClick(item.id);
                });
            }

            container.appendChild(row);
        }
    }

    renderItems(menuEl, items);

    overlay.appendChild(menuEl);
    document.body.appendChild(overlay);
    // visibility:hidden until placed below → registers no hole until then.
    overlayHandles.push(registerPaneOverlay(menuEl));

    // Position at the cursor via the shared framework — flip/shift/size keep
    // the menu on-screen near window edges. Unlike FlyoutMenu/Popover, a
    // right-click context menu MUST appear where the user clicked, so
    // `avoidNativePanes` is OFF: it is *expected* to land over a browser
    // pane. The `data-pane-overlay` clip reveals it through the native pane;
    // holding it visibility:hidden until placed means the clip rect registers
    // once, at the final position — no flapping, no stale-rect black artifact.
    void computeMenuPosition(
        {
            anchor: { x: position.x, y: position.y },
            placement: "bottom-start",
            avoidNativePanes: false,
        },
        menuEl,
    ).then((pos) => {
        if (!menuEl.isConnected) return;
        applyMenuPosition(menuEl, pos);
        menuEl.style.visibility = "";
        assertMenuInPaintableArea(menuEl, "context-menu");
    }).catch(() => {
        // computeMenuPosition should not throw; if it does, fall back to the
        // raw cursor position rather than leaving the menu invisible.
        if (menuEl.isConnected) menuEl.style.visibility = "";
    });
}

/** `AppApi.showContextMenu` and `AppApi.onContextMenuClick`, backed by the JS
 *  overlay above. */
export function jsContextMenuApi(): Pick<AppApi, "showContextMenu" | "onContextMenuClick"> {
    return {
        showContextMenu: (_workspaceId: string, menu?: NativeContextMenuItem[], position?: { x: number; y: number }) => {
            if (!menu || menu.length === 0) return;
            showJsContextMenu(menu, position ?? { x: 0, y: 0 }, contextMenuClickCallback);
        },
        onContextMenuClick: (callback: (id: string) => void) => {
            contextMenuClickCallback = callback;
        },
    };
}
