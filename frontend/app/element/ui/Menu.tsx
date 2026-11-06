// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Menu: the one anchored menu. It hangs from an element, a rect or a point, opens upward and flips
 * down only when there is no room above (the placement default; `AnchoredPopover` does the placing,
 * the chrome zoom and the airspace cut), closes on a pick, an outside press or Esc, and is driven
 * from the keyboard. A row-actions menu, a right-click menu and a short choice ("already open:
 * fork or switch?") are all this one component with different items. A row with `submenu` opens
 * another level beside it: the same panel, anchored to the row.
 */

import { AnchoredPopover } from "@/app/element/anchored-popover";
import { menuIconClass } from "@/app/util/menu-icons";
import { createPeerRegistry, createSubmenuHover, type SubmenuHoverController } from "@/app/util/submenu-hover";
import type { Placement } from "@floating-ui/dom";
import clsx from "clsx";
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show, type JSX } from "solid-js";
import "./menu.scss";

export interface MenuEntry {
    label: string;
    /** A Font Awesome name, drawn in the leading slot (`"regular@window-maximize"` for the regular set), or an element
     *  drawn there as it is (a site's favicon or badge; it brings its own `menu-item-icon` slot). */
    icon?: string | JSX.Element;
    /** A choice the row toggles (`checkbox`) or one of a group (`radio`): the leading slot shows a check while `checked`. */
    type?: "checkbox" | "radio";
    checked?: boolean;
    onSelect?: () => void;
    tone?: "danger";
    disabled?: boolean;
    /** Shown right-aligned, already formatted ("Ctrl+P"). */
    shortcut?: string;
    /** A hairline between groups; no other field applies. */
    divider?: boolean;
    testId?: string;
    /** Secondary, muted text after the label. */
    sublabel?: string;
    /** An exact-hue square before the label (a palette colour), in place of the icon. */
    swatchColor?: string;
    /** More inside the label (a second line under it); the row's `class` styles it. */
    detail?: JSX.Element;
    /** After the label: a mark at the row's end (the tab's own identity). */
    trailing?: JSX.Element;
    /** Extra classes for the row. */
    class?: string;
    /** A submenu: the row opens it (hovering, Right or Enter) and has no pick of its own. */
    submenu?: MenuEntry[];
}

/** An element (followed live), a rect, or a point (a right-click). */
export type MenuAnchor = Element | DOMRect | { x: number; y: number };

export interface MenuProps {
    anchor: MenuAnchor | null | undefined;
    items: MenuEntry[];
    /** A line above the rows that says what the menu is about; not a row. */
    header?: JSX.Element;
    /** Upward by default; it flips down when the room above is too small. */
    placement?: Placement;
    /** Keep the leading slot on every row when any row has an icon, so the labels line up. */
    alignIcons?: boolean;
    /** Keep the menu inside this element (a browser pane) when it fits; with `alignToAnchorSide`, open it from the side its anchor is on. */
    containWithin?: () => HTMLElement | null | undefined;
    alignToAnchorSide?: boolean;
    /** A pick, an outside press or Esc. The owner unmounts the menu. */
    onClose: () => void;
    "aria-label"?: string;
    class?: string;
    testId?: string;
    /** A menu whose pick is the person's approval (sign in with a saved password):
     *  marks it `data-operator-only`, which agents' UI query and click leave out and
     *  refuse (`crates/cef/src/browser_api/scripts/query.js`), as `Modal`'s `operatorOnly`. */
    operatorOnly?: boolean;
}

const asAnchor = (a: MenuAnchor | null | undefined): Element | DOMRect | undefined => {
    if (!a) return undefined;
    if (a instanceof Element || a instanceof DOMRect) return a;
    return new DOMRect(a.x, a.y, 0, 0);
};

/** One level of rows: the root menu, or a submenu anchored to the row that opened it. */
interface MenuPanelProps {
    items: MenuEntry[];
    anchor: Element | DOMRect | undefined;
    placement: Placement;
    header?: JSX.Element;
    alignIcons?: boolean;
    containWithin?: () => HTMLElement | null | undefined;
    alignToAnchorSide?: boolean;
    /** A pick: closes the whole menu, then runs the pick. */
    onPick: () => void;
    /** An outside press or Esc on this level (the root closes the menu; a submenu closes itself). */
    onDismiss: (reason: "outside" | "escape") => void;
    /** Left, in a submenu: back to the level above. */
    onBack?: () => void;
    /** Focus the first row when shown (a submenu opened from the keyboard). */
    focusFirst?: boolean;
    /** Changes each time the keyboard asks for this (already open) submenu: focus it again. */
    focusToken?: number;
    /** The panel's body, and when the pointer enters or leaves it (a submenu's hover intent). */
    onBody?: (el: HTMLDivElement) => void;
    /** Hands the level above a test for "a press here is inside this menu" (this panel, or a level below it). */
    onKeep?: (inside: (target: Node) => boolean) => void;
    onPointer?: (inside: boolean, e: MouseEvent) => void;
    "aria-label"?: string;
    class?: string;
    testId?: string;
    operatorOnly?: boolean;
}

/** The Font Awesome class of a named icon; an element icon brings its own slot. */
const namedIcon = (e: MenuEntry): string | null => (typeof e.icon === "string" ? menuIconClass(e.icon) : null);

/** Whether a row draws an icon in the leading slot (a named one, or an element). */
const hasLeadIcon = (e: MenuEntry): boolean => !e.divider && !e.swatchColor && (typeof e.icon === "string" ? !!menuIconClass(e.icon) : !!e.icon);

const roleOf = (e: MenuEntry) => (e.type === "checkbox" ? "menuitemcheckbox" : e.type === "radio" ? "menuitemradio" : "menuitem");

/**
 * Which row of a level has its submenu open, and the hover intent that opens and closes it: hovering a row with
 * a submenu opens it after a short delay, entering any other row closes it, and a pick or the keyboard opens it
 * at once.
 */
function useSubmenus() {
    const [open, setOpen] = createSignal<{ index: number; focus: boolean; token: number } | null>(null);
    let tokens = 0;
    const peers = createPeerRegistry();
    const hovers = new Map<number, SubmenuHoverController>();
    let hoveredRow: number | null = null;
    onCleanup(() => hovers.forEach((h) => h.dispose()));
    const hoverFor = (i: number): SubmenuHoverController => {
        let h = hovers.get(i);
        if (!h) {
            h = createSubmenuHover({
                closeOnLeave: false,
                onOpen: () => setOpen({ index: i, focus: false, token: ++tokens }),
                onClose: () => setOpen((cur) => (cur?.index === i ? null : cur)),
            });
            hovers.set(i, h);
            peers.register(String(i), h);
        }
        return h;
    };
    return {
        open,
        hovers,
        hoverFor,
        // The open submenu of another row stays while the pointer is still crossing toward it; this row's own
        // opens once that has closed, if the pointer is still on the row.
        enter: (i: number, hasSubmenu: boolean, e: MouseEvent) => {
            hoveredRow = i;
            peers.closeOthers(String(i), e, () => {
                if (hasSubmenu && hoveredRow === i) hoverFor(i).onTriggerEnter();
            });
        },
        leave: (i: number, e: MouseEvent) => {
            if (hoveredRow === i) hoveredRow = null;
            hovers.get(i)?.onTriggerLeave(e);
        },
        openNow: (i: number, focus: boolean) => {
            peers.closeOthers(String(i));
            hoverFor(i).openNow();
            setOpen({ index: i, focus, token: ++tokens });
        },
        close: () => {
            const cur = open();
            if (cur) hovers.get(cur.index)?.close();
            setOpen(null);
        },
    };
}

interface MenuRowProps {
    entry: MenuEntry;
    active: boolean;
    expanded: boolean;
    reserve: boolean;
    ref: (el: HTMLElement) => void;
    onEnter: (e: MouseEvent) => void;
    onLeave: (e: MouseEvent) => void;
    onPick: () => void;
}

/** One row: the lead slot, label, sublabel, shortcut and the chevron of a submenu. */
function MenuRow(props: MenuRowProps): JSX.Element {
    const e = () => props.entry;
    return (
        <div
            ref={props.ref}
            role={roleOf(e())}
            class={clsx("menu-item ui-menu-item", {
                active: props.active,
                "ui-menu-item--danger": e().tone === "danger",
                "ui-menu-item--disabled": e().disabled,
            }, e().class)}
            aria-checked={e().type ? !!e().checked : undefined}
            aria-disabled={e().disabled || undefined}
            aria-haspopup={e().submenu ? "menu" : undefined}
            aria-expanded={e().submenu ? props.expanded : undefined}
            data-testid={e().testId}
            onMouseEnter={props.onEnter}
            onMouseLeave={props.onLeave}
            onClick={props.onPick}
        >
            <MenuRowLead entry={e()} reserve={props.reserve} />
            <span class="label">
                {e().label}
                {e().detail}
            </span>
            {e().trailing}
            <Show when={e().sublabel}>
                <span class="menu-item-sublabel">{e().sublabel}</span>
            </Show>
            <Show when={e().shortcut}>
                <span class="menu-item-shortcut">{e().shortcut}</span>
            </Show>
            <Show when={e().submenu}>
                <i class="fa-solid fa-chevron-right menu-item-chevron" aria-hidden="true" />
            </Show>
        </div>
    );
}

/** The active row, the keys that move it, and what a pick does (open a submenu, or close the menu and run the row). */
function useMenuNav(props: MenuPanelProps, sub: ReturnType<typeof useSubmenus>, body: () => HTMLDivElement | undefined) {
    const [active, setActive] = createSignal(-1);
    const pickable = (i: number) => {
        const e = props.items[i];
        return !!e && !e.divider && !e.disabled;
    };
    const step = (from: number, dir: 1 | -1): number => {
        const n = props.items.length;
        for (let k = 1; k <= n; k++) {
            const i = (((from + dir * k) % n) + n) % n;
            if (pickable(i)) return i;
        }
        return from;
    };
    const select = (i: number) => {
        if (!pickable(i)) return;
        const entry = props.items[i];
        if (entry.submenu) return sub.openNow(i, false);
        props.onPick();
        entry.onSelect?.();
    };
    const onKeyDown = (e: KeyboardEvent) => {
        const cur = active();
        if (e.key === "ArrowDown") setActive(step(cur, 1));
        else if (e.key === "ArrowUp") setActive(step(cur < 0 ? props.items.length : cur, -1));
        else if (e.key === "Home") setActive(step(-1, 1));
        else if (e.key === "End") setActive(step(props.items.length, -1));
        else if ((e.key === "ArrowRight" || e.key === "Enter" || e.key === " ") && props.items[cur]?.submenu && pickable(cur)) sub.openNow(cur, true);
        else if (e.key === "Enter" || e.key === " ") select(cur);
        else if (e.key === "ArrowLeft" && props.onBack) props.onBack();
        else return;
        e.preventDefault();
        e.stopPropagation();
    };
    // A browser does not focus what is still hidden, and the panel is hidden until it is placed.
    // A submenu that hover opened leaves the keys in the level above until Right or Enter asks for them.
    const focusBody = () => {
        if (props.onBack && !props.focusFirst) return;
        body()?.focus({ preventScroll: true });
        if (props.focusFirst) setActive(step(-1, 1));
    };
    // Right or Enter on a submenu that hover already opened: it is shown, so move the keys into it now.
    createEffect(on(() => props.focusToken, () => props.focusFirst && focusBody(), { defer: true }));
    return { active, setActive, pickable, select, onKeyDown, focusBody };
}

function MenuPanel(props: MenuPanelProps): JSX.Element {
    const sub = useSubmenus();
    let body: HTMLDivElement | undefined;
    let subKeep: ((target: Node) => boolean) | undefined;
    const nav = useMenuNav(props, sub, () => body);
    const rows: (HTMLElement | undefined)[] = [];
    const reserveIcons = () => !!props.alignIcons && props.items.some(hasLeadIcon);
    // A submenu that had the keys hands them back to this level when it closes, by any path (Left, Esc, an outside
    // press, the pointer entering another row); otherwise focus falls to <body> and the arrows stop working.
    let subFocused = false;
    createEffect(
        on(
            () => sub.open() !== null,
            (isOpen, wasOpen) => {
                if (wasOpen && !isOpen && subFocused) queueMicrotask(() => body?.focus({ preventScroll: true }));
                if (!isOpen) subFocused = false;
            },
            { defer: true },
        ),
    );
    // One panel per open row: moving from one submenu to another builds a new panel, anchored to its own row.
    const openIndex = createMemo(() => sub.open()?.index ?? -1);

    return (
        <>
            <AnchoredPopover
                anchor={props.anchor}
                placement={props.placement}
                motion="fadeScale"
                onShown={nav.focusBody}
                containWithin={props.containWithin} alignToAnchorSide={props.alignToAnchorSide} fitHeight
                shellClass="anchored-popover--menu"
                class={clsx("menu ui-menu", props.onBack && "sub-menu", props.class)}
                role="menu"
                aria-label={props["aria-label"]}
                // Esc while a submenu is open belongs to the submenu (it closes only itself); a press inside it is inside.
                onDismiss={(reason) => {
                    if (reason === "escape" && sub.open()) return;
                    props.onDismiss(reason);
                }}
                keepOpenOn={(t) => !!subKeep?.(t)}
                ref={(el) => {
                    body = el;
                    el.tabIndex = -1;
                    if (props.operatorOnly) el.setAttribute("data-operator-only", "");
                    el.addEventListener("keydown", nav.onKeyDown);
                    if (props.testId) el.dataset.testid = props.testId;
                    props.onBody?.(el);
                    props.onKeep?.((t) => el.contains(t) || !!subKeep?.(t));
                    el.addEventListener("mouseenter", (e) => props.onPointer?.(true, e));
                    el.addEventListener("mouseleave", (e) => props.onPointer?.(false, e));
                }}
            >
                <Show when={props.header}>
                    <div class="ui-menu-header">{props.header}</div>
                </Show>
                <For each={props.items}>
                    {(entry, i) => (
                        <Show when={!entry.divider} fallback={<div class="menu-divider" aria-hidden="true" />}>
                            <MenuRow
                                entry={entry}
                                active={nav.active() === i()}
                                expanded={sub.open()?.index === i()}
                                reserve={reserveIcons()}
                                ref={(el) => (rows[i()] = el)}
                                onEnter={(e) => {
                                    if (nav.pickable(i())) nav.setActive(i());
                                    sub.enter(i(), !!entry.submenu && nav.pickable(i()), e);
                                }}
                                onLeave={(e) => sub.leave(i(), e)}
                                onPick={() => nav.select(i())}
                            />
                        </Show>
                    )}
                </For>
            </AnchoredPopover>
            <For each={openIndex() >= 0 ? [openIndex()] : []}>
                {(index) => (
                    <MenuPanel
                        items={props.items[index].submenu!}
                        anchor={rows[index]}
                        placement="right-start"
                        alignIcons={props.alignIcons}
                        focusFirst={sub.open()?.focus}
                        focusToken={sub.open()?.token}
                        onPick={props.onPick}
                        onDismiss={() => sub.close()}
                        onBack={() => sub.close()}
                        onBody={(el) => {
                            sub.hovers.get(index)?.setSubmenuEl(el);
                            el.addEventListener("focusin", () => (subFocused = true));
                        }}
                        onKeep={(inside) => (subKeep = inside)}
                        onPointer={(inside, e) => (inside ? sub.hovers.get(index)?.onSubmenuEnter() : sub.hovers.get(index)?.onSubmenuLeave(e))}
                    />
                )}
            </For>
        </>
    );
}

/** The slot before a label: a check (a choice), or an icon, or (when the level aligns icons) an empty slot; then a swatch, if any. */
function MenuRowLead(props: { entry: MenuEntry; reserve: boolean }): JSX.Element {
    return (
        <>
            <Show
                when={!props.entry.type}
                fallback={<i class={clsx("fa-solid fa-fw menu-item-icon menu-item-check", props.entry.checked && "fa-check")} aria-hidden="true" />}
            >
                <Show when={!props.entry.swatchColor}>
                    <Show
                        when={namedIcon(props.entry)}
                        fallback={
                            <Show when={props.entry.icon} fallback={<Show when={props.reserve}><span class="menu-item-icon" aria-hidden="true" /></Show>}>
                                {props.entry.icon as JSX.Element}
                            </Show>
                        }
                    >
                        {(cls) => <i class={cls()} aria-hidden="true" />}
                    </Show>
                </Show>
            </Show>
            <Show when={props.entry.swatchColor}>
                {(color) => <span class="menu-item-swatch" style={{ "background-color": color() }} />}
            </Show>
        </>
    );
}

export const Menu = (props: MenuProps): JSX.Element => (
    <MenuPanel
        items={props.items}
        anchor={asAnchor(props.anchor)}
        placement={props.placement ?? "top-start"}
        header={props.header}
        alignIcons={props.alignIcons}
        containWithin={props.containWithin}
        alignToAnchorSide={props.alignToAnchorSide}
        onPick={props.onClose}
        onDismiss={() => props.onClose()}
        aria-label={props["aria-label"]}
        class={props.class}
        testId={props.testId}
        operatorOnly={props.operatorOnly}
    />
);
