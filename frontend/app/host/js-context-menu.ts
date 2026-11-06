// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The context menu as a positioned HTML overlay: what `AppApi.showContextMenu`
// shows for a host with no native menu of its own. Plain DOM, no host calls,
// so every host uses this one (`jsContextMenuApi`). Moved out of the CEF shim
// (app/host/cef-api.ts), which was its only user.

import { registerPaneOverlay, type PaneOverlayHandle } from "@/app/platform/pane-overlay";
import "../element/anchored-popover.scss";
import {
    assertMenuInPaintableArea,
    computeMenuPosition,
    type MenuPositionResult,
} from "@/app/util/menu-position";
import { menuIconClass } from "@/app/util/menu-icons";
import { createSubmenuHover, type SubmenuHoverController } from "@/app/util/submenu-hover";
import { Z_JS_MENU } from "@/app/zindex";

// Context menu click callback — registered by onContextMenuClick, called by showJsContextMenu.
let contextMenuClickCallback: ((id: string) => void) | null = null;

// Close function of the currently open showJsContextMenu, if any.
let activeContextMenuClose: (() => void) | null = null;

// Replaces the open menu's items in place, if a menu is open.
let activeContextMenuUpdate: ((items: NativeContextMenuItem[]) => void) | null = null;

// Called whenever a menu closes, by any path (click, Escape, outside click, another menu).
const closeListeners = new Set<() => void>();

/** Whether a JS context menu is open now. */
export function isJsContextMenuOpen(): boolean {
    return activeContextMenuClose != null;
}

/** Close the open JS context menu (if any), releasing its pane-overlay holes. */
export function closeJsContextMenu(): void {
    activeContextMenuClose?.();
}

/**
 * Replace the items of the menu that is open, where it is: a live menu (the pane "+" picker)
 * follows a config change without closing. A submenu that was open stays open if a submenu
 * with the same label is still there. Returns false when no menu is open.
 */
export function updateJsContextMenu(items: NativeContextMenuItem[]): boolean {
    if (!activeContextMenuUpdate) return false;
    activeContextMenuUpdate(items);
    return true;
}

/** Run `listener` the next time any JS context menu closes. Returns an unsubscribe. */
export function onJsContextMenuClosed(listener: () => void): () => void {
    closeListeners.add(listener);
    return () => closeListeners.delete(listener);
}

/**
 * One level of a menu (the menu, or a submenu): an unzoomed `position: fixed` shell, placed in real pixels, around
 * a body that carries chrome zoom and the menu's look: the shape AnchoredPopover uses (REPORT_CHROME_ZOOM_POPOVERS_
 * 2026_10_02.md section 4). Every level is a sibling in the overlay, never nested in the row that opens it: a fixed
 * child of a zoomed body has its left/top scaled by the zoom a second time.
 */
function createLevel(sub: boolean): { shell: HTMLDivElement; body: HTMLDivElement } {
    const shell = document.createElement("div");
    shell.className = "anchored-popover anchored-popover--menu";
    shell.setAttribute("data-pane-overlay", "");
    const body = document.createElement("div");
    body.className = `menu${sub ? " sub-menu" : ""} anchored-popover-body anchored-popover-body--chrome-zoom`;
    shell.appendChild(body);
    return { shell, body };
}

/**
 * Apply a computed MenuPositionResult to a level: the shell's fixed left/top, plus the size() max-height cap on
 * the body (in its own zoomed lengths) so a menu taller than the free space scrolls internally instead of being
 * placed partly outside. max-width is deliberately NOT applied: an inline max-width would override (and can
 * loosen) the .menu 400px CSS cap, and horizontal fit is already guaranteed by flip+shift for menus at or under
 * that cap. The rows start at the top of a capped body through `.anchored-popover--menu > .menu`.
 */
function applyMenuPosition(shell: HTMLElement, body: HTMLElement, pos: MenuPositionResult) {
    shell.style.position = "fixed";
    if (pos.style.left != null) shell.style.left = String(pos.style.left);
    if (pos.style.top != null) shell.style.top = String(pos.style.top);
    body.style.maxHeight = `calc(${pos.maxHeight}px / var(--zoomfactor, 1))`;
    body.style.overflowY = "auto";
}

/** What one open menu and the functions that serve it share. */
interface MenuState {
    position: { x: number; y: number };
    onClick: ((id: string) => void) | null;
    opener: HTMLElement | null;
    overlay: HTMLDivElement;
    /** The menu's shell (placed) and body (its rows). */
    menuShell: HTMLDivElement;
    menuEl: HTMLDivElement;
    /** The submenu shells of every level, siblings of `menuShell` in the overlay. */
    subShells: HTMLElement[];
    overlayHandles: PaneOverlayHandle[];
    submenuHandles: PaneOverlayHandle[];
    topLevelSubmenus: Map<string, { hover: SubmenuHoverController; sub: HTMLElement }>;
    onDocMouseDown: (e: MouseEvent) => void;
    onDocKeyDown: (e: KeyboardEvent) => void;
    close: () => void;
}

function createMenuState(position: { x: number; y: number }, onClick: ((id: string) => void) | null): MenuState {
    // Where the caret was when the menu opened (a terminal's textarea, an
    // input), so a menu that closes without moving focus elsewhere hands it
    // back. SPEC_CONTEXT_MENU_PASTE_KEEPS_TERMINAL_FOCUS_2026_10_07.md.
    const opener = document.activeElement instanceof HTMLElement && document.activeElement !== document.body
        ? document.activeElement
        : null;

    const overlay = document.createElement("div");
    overlay.id = "cef-context-menu-overlay";
    Object.assign(overlay.style, {
        position: "fixed", inset: "0", zIndex: String(Z_JS_MENU),
    });

    // Every close path goes through closeMenu(): it releases the pane-overlay
    // registrations (the menu and each submenu punch a hole through any
    // browser pane they overlap) and the document listeners before removing
    // the DOM. A leaked registration would leave a see-through,
    // click-through hole in the pane after the menu is gone.
    const overlayHandles: PaneOverlayHandle[] = [];
    // Submenu holes, released and rebuilt when a live menu's items change.
    const submenuHandles: PaneOverlayHandle[] = [];
    // Top-level submenu rows by label, to reopen the one that was open after an update.
    const topLevelSubmenus = new Map<string, { hover: SubmenuHoverController; sub: HTMLElement }>();

    const { shell: menuShell, body: menuEl } = createLevel(false);
    menuShell.style.left = `${position.x}px`;
    menuShell.style.top = `${position.y}px`;
    // Hidden until computeMenuPosition places it — avoids a one-frame flash
    // at the cursor and stops the pane-overlay clip from firing for a
    // not-yet-positioned menu (visibility:hidden de-registers the rect).
    menuShell.style.visibility = "hidden";

    const s: MenuState = {
        position,
        onClick,
        opener,
        overlay,
        menuShell,
        menuEl,
        subShells: [],
        overlayHandles,
        submenuHandles,
        topLevelSubmenus,
        onDocMouseDown: (e: MouseEvent) => {
            // The backdrop covers the whole window, so a real click lands on it.
            // What reaches here from outside is the synthetic body mousedown that
            // browser-pane-outside-click-bridge.ts fires for a click inside a
            // native browser pane — treat it as an outside click.
            if (!(e.target instanceof Node) || !overlay.contains(e.target)) closeMenu(s);
        },
        onDocKeyDown: (e: KeyboardEvent) => {
            if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                closeMenu(s);
            }
        },
        close: () => closeMenu(s),
    };
    return s;
}

function closeMenu(s: MenuState) {
    if (activeContextMenuClose === s.close) {
        activeContextMenuClose = null;
        activeContextMenuUpdate = null;
    }
    for (const h of s.overlayHandles) h.release();
    s.overlayHandles.length = 0;
    for (const h of s.submenuHandles) h.release();
    s.submenuHandles.length = 0;
    for (const listener of [...closeListeners]) listener();
    document.removeEventListener("mousedown", s.onDocMouseDown, true);
    document.removeEventListener("keydown", s.onDocKeyDown, true);
    s.overlay.remove();
    // After the chosen item has run (it runs right after closeMenu): if
    // focus fell to <body> or went with the removed menu, give it back to
    // the opener. An item that moved focus on purpose (a split's new pane,
    // Inspect) keeps it.
    queueMicrotask(() => {
        const active = document.activeElement;
        const lost = active == null || active === document.body || s.overlay.contains(active);
        if (lost && s.opener?.isConnected) s.opener.focus({ preventScroll: true });
    });
}

function installDismissal(s: MenuState) {
    activeContextMenuClose = s.close;
    document.addEventListener("mousedown", s.onDocMouseDown, true);
    document.addEventListener("keydown", s.onDocKeyDown, true);

    s.overlay.addEventListener("mousedown", (e) => {
        // A press on the menu or its backdrop must not take focus: the browser
        // otherwise blurs the focused element (a terminal's textarea) to
        // <body>, and after "Paste" the user's next Enter went nowhere.
        // The click still fires; only the focus change is cancelled.
        e.preventDefault();
        if (e.target === s.overlay) closeMenu(s);
    });
}

function appendLeadingMark(row: HTMLElement, item: NativeContextMenuItem, reserveIconSlot: boolean) {
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
    } else {
        // The same leading slot holds the row's icon (SPEC_CONTEXT_MENU_ICONS_2026_10_25.md
        // §3). A menu level where any row has one reserves the slot on every row, so the
        // labels line up; a level with none is drawn as before. Decoration only: the label
        // is the row's name.
        const iconClass = item.swatchColor ? null : menuIconClass(item.icon);
        if (iconClass) {
            const icon = document.createElement("i");
            icon.className = iconClass;
            icon.setAttribute("aria-hidden", "true");
            row.appendChild(icon);
        } else if (reserveIconSlot && !item.swatchColor) {
            const spacer = document.createElement("span");
            spacer.className = "menu-item-icon";
            spacer.setAttribute("aria-hidden", "true");
            row.appendChild(spacer);
        }
    }
}

function appendSwatchAndLabels(row: HTMLElement, item: NativeContextMenuItem) {
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
}

function openSubmenu(row: HTMLElement, sub: HTMLElement, body: HTMLElement) {
    sub.style.visibility = "hidden";
    sub.style.display = "";
    // Deferred one rAF (mirrors flyoutmenu.tsx's SubMenu /
    // registerSubMenu) so the display:none→"" reflow has
    // settled before anything gets measured.
    requestAnimationFrame(() => {
        if (!sub.isConnected || sub.style.display === "none") return;
        // computeMenuPosition computes coordinates for `strategy:"fixed"`, and floating-ui resolves the floating
        // element's offset parent from its CURRENT position at measurement time: switch to fixed BEFORE measuring
        // (FlyoutMenu's Solid sibling starts at `position:fixed;left:0px;top:0px`), or the coordinates are
        // resolved against the wrong containing block (live 2026-08-13: left:121px, top:-393px every time).
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
            applyMenuPosition(sub, body, pos);
            sub.style.visibility = "";
            assertMenuInPaintableArea(sub, "context-submenu");
        }).catch(() => {
            if (!sub.isConnected) return;
            // Placement failed: sit at the row's right edge, in real pixels.
            const r = row.getBoundingClientRect();
            sub.style.left = `${r.right}px`;
            sub.style.top = `${r.top}px`;
            sub.style.visibility = "";
        });
    });
}

function attachSubmenu(
    s: MenuState,
    row: HTMLElement,
    item: NativeContextMenuItem,
    peers: SubmenuHoverController[],
    topLevel: boolean
) {
    const arrow = document.createElement("i");
    arrow.className = "fa-solid fa-chevron-right";
    row.appendChild(arrow);

    // The submenu is its own level in the overlay (see createLevel), not a child of the row; the hover logic
    // below works on its geometry and events, so it does not need to be nested.
    const { shell: sub, body: subBody } = createLevel(true);
    sub.style.display = "none";
    sub.style.left = "0px";
    sub.style.top = "0px";
    // In before its own rows are rendered, so a submenu of this one sits above it.
    s.overlay.appendChild(sub);
    s.subShells.push(sub);
    const childPeers = renderItems(s, subBody, item.submenu);
    // display:none until hovered → no hole until it's shown and
    // placed; the style observer picks up each open/close.
    s.submenuHandles.push(registerPaneOverlay(sub, { keys: true }));
    // Positioned through the shared framework, like the top-level
    // menu: anchored to the row, preferring right-start — flip()
    // sends it left of the parent near the right edge, shift()
    // pulls it up near the bottom, size() caps it when taller than
    // the free space. Held visibility:hidden until
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
        // Stays open when the pointer leaves the menu; a sibling
        // row's mouseenter (peer close below) closes it.
        closeOnLeave: false,
        onOpen: () => openSubmenu(row, sub, subBody),
        onClose: () => {
            sub.style.display = "none";
            // Its own submenus are siblings now, not hidden with it.
            for (const peer of childPeers) peer.close();
        },
    });
    // `sub` reports a zero rect via getBoundingClientRect() while
    // display:none, which the controller treats as "no geometry
    // yet" — safe to register once, up front.
    hover.setSubmenuEl(sub);
    peers.push(hover);
    if (topLevel && item.label) s.topLevelSubmenus.set(item.label, { hover, sub });
    row.addEventListener("mouseenter", () => {
        for (const peer of peers) {
            if (peer !== hover) peer.close();
        }
        hover.onTriggerEnter();
    });
    row.addEventListener("mouseleave", (e) => hover.onTriggerLeave(e as MouseEvent));
    sub.addEventListener("mouseenter", () => hover.onSubmenuEnter());
    sub.addEventListener("mouseleave", (e) => hover.onSubmenuLeave(e as MouseEvent));
}

function attachPlainRow(s: MenuState, row: HTMLElement, item: NativeContextMenuItem, peers: SubmenuHoverController[]) {
    // A plain (no-submenu) row is still an explicit new selection —
    // entering it closes any open peer submenu immediately too.
    row.addEventListener("mouseenter", () => {
        for (const peer of peers) peer.close();
    });
    row.addEventListener("click", () => {
        closeMenu(s);
        if (item.id && s.onClick) s.onClick(item.id);
    });
}

function renderItems(s: MenuState, container: HTMLElement, itemList: NativeContextMenuItem[]): SubmenuHoverController[] {
    const topLevel = container === s.menuEl;
    // Peers (same-level submenu-bearing rows) close each other instantly
    // on entry — matches the old zero-delay mouseleave's implicit
    // sibling-closing side effect, which createSubmenuHover's open-delay/
    // safe-triangle close otherwise silently drops (reagent P1 on
    // PR #2525; the terminal's Themes/Font Size/Terminal Zoom/
    // Transparency submenus were the concrete case). One list
    // per renderItems call — each call is exactly one menu level, so
    // nested submenus never reach across levels to close an ancestor.
    const peers: SubmenuHoverController[] = [];
    const reserveIconSlot = itemList.some((i) => i.type !== "separator" && i.visible !== false && !i.swatchColor && !!menuIconClass(i.icon));
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
        appendLeadingMark(row, item, reserveIconSlot);
        appendSwatchAndLabels(row, item);

        if (item.submenu && item.submenu.length > 0) {
            attachSubmenu(s, row, item, peers, topLevel);
        } else if (item.enabled !== false) {
            attachPlainRow(s, row, item, peers);
        }

        container.appendChild(row);
    }
    return peers;
}

function placeMenu(s: MenuState) {
    // Position at the cursor via the shared framework — flip/shift/size keep
    // the menu on-screen near window edges. Unlike FlyoutMenu/Popover, a
    // right-click context menu MUST appear where the user clicked, so
    // `avoidNativePanes` is OFF: it is *expected* to land over a browser
    // pane. The `data-pane-overlay` clip reveals it through the native pane;
    // holding it visibility:hidden until placed means the clip rect registers
    // once, at the final position — no flapping, no stale-rect black artifact.
    void computeMenuPosition(
        {
            anchor: { x: s.position.x, y: s.position.y },
            placement: "bottom-start",
            avoidNativePanes: false,
        },
        s.menuShell,
    ).then((pos) => {
        if (!s.menuShell.isConnected) return;
        applyMenuPosition(s.menuShell, s.menuEl, pos);
        s.menuShell.style.visibility = "";
        assertMenuInPaintableArea(s.menuShell, "context-menu");
    }).catch(() => {
        // computeMenuPosition should not throw; if it does, fall back to the
        // raw cursor position rather than leaving the menu invisible.
        if (s.menuShell.isConnected) s.menuShell.style.visibility = "";
    });
}

function updateItems(s: MenuState, next: NativeContextMenuItem[]) {
    if (!s.menuEl.isConnected) return;
    let openLabel: string | null = null;
    for (const [label, { hover, sub }] of s.topLevelSubmenus) {
        if (sub.style.display !== "none") openLabel = label;
        hover.dispose();
    }
    s.topLevelSubmenus.clear();
    for (const h of s.submenuHandles) h.release();
    s.submenuHandles.length = 0;
    for (const sub of s.subShells) sub.remove();
    s.subShells.length = 0;
    s.menuEl.replaceChildren();
    renderItems(s, s.menuEl, next);
    // Same anchor, new height: re-place so a longer list still fits the window.
    s.menuEl.style.maxHeight = "";
    placeMenu(s);
    if (openLabel) s.topLevelSubmenus.get(openLabel)?.hover.openNow();
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

    const s = createMenuState(position, onClick);
    installDismissal(s);
    // The menu goes in before its rows are rendered, so the submenus (appended while rendering) sit above it.
    s.overlay.appendChild(s.menuShell);
    renderItems(s, s.menuEl, items);

    document.body.appendChild(s.overlay);
    // visibility:hidden until placed below → registers no hole until then.
    s.overlayHandles.push(registerPaneOverlay(s.menuShell, { keys: true }));

    placeMenu(s);

    activeContextMenuUpdate = (next: NativeContextMenuItem[]) => updateItems(s, next);
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
