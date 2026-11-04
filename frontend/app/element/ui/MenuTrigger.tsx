// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MenuTrigger: a button (or any element) that opens a `Menu` hanging from it, and closes it on a second
 * press, a pick, an outside press or Esc. It is `FlyoutMenu`'s open/close without its rows: the rows are
 * the kit Menu's. Focus goes back to the trigger when the menu closes without a pick that moved it.
 */

import { Menu, type MenuEntry, type MenuProps } from "@/app/element/ui/Menu";
import { createSignal, Show, type JSX } from "solid-js";

export interface MenuTriggerProps extends Pick<MenuProps, "placement" | "alignIcons" | "containWithin" | "alignToAnchorSide" | "aria-label" | "class" | "testId"> {
    items: MenuEntry[];
    /** Called with true as the menu opens and false as it closes (load what the rows show). */
    onOpenChange?: (open: boolean) => void;
    /** The element that opens it, drawn inside the anchor. */
    children: JSX.Element;
}

export function MenuTrigger(props: MenuTriggerProps): JSX.Element {
    const [open, setOpen] = createSignal(false);
    let anchor: HTMLDivElement | undefined;
    let opener: HTMLElement | null = null;
    const set = (next: boolean) => {
        if (open() === next) return;
        setOpen(next);
        props.onOpenChange?.(next);
    };
    const toggle = () => {
        if (!open()) {
            const active = document.activeElement;
            opener = active instanceof HTMLElement && anchor?.contains(active) ? active : (anchor?.querySelector<HTMLElement>("button, [tabindex]") ?? null);
        }
        set(!open());
    };
    const close = () => {
        set(false);
        // After a pick has run: focus that fell to <body> goes back to the trigger. A pick that moved it keeps it.
        queueMicrotask(() => {
            const active = document.activeElement;
            if ((active == null || active === document.body) && opener?.isConnected) opener.focus({ preventScroll: true });
        });
    };
    return (
        <>
            <div
                ref={anchor}
                class="menu-anchor"
                data-drag-region="false"
                onPointerDown={() => {
                    // Browser panes capture their freeze-frame now, ~100ms before the click opens the menu, so the
                    // airspace cut under it releases at once (browser-view.tsx).
                    if (!open()) window.dispatchEvent(new CustomEvent("pane-freeze-prewarm"));
                }}
                onClick={toggle}
            >
                {props.children}
            </div>
            <Show when={open()}>
                <Menu
                    anchor={anchor}
                    items={props.items}
                    placement={props.placement ?? "bottom-start"}
                    alignIcons={props.alignIcons}
                    containWithin={props.containWithin}
                    alignToAnchorSide={props.alignToAnchorSide}
                    aria-label={props["aria-label"]}
                    class={props.class}
                    testId={props.testId}
                    onClose={close}
                />
            </Show>
        </>
    );
}
