// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cn } from "@/util/util";
import {
    autoUpdate,
    computePosition,
    flip,
    offset,
    shift,
    type Placement,
} from "@floating-ui/dom";
import { createEffect, createSignal, JSX, onCleanup, Show } from "solid-js";
import { Portal } from "solid-js/web";
import type { Properties as CSSProperties } from "csstype";

interface TooltipProps {
    children?: JSX.Element;
    content?: JSX.Element;
    placement?: "top" | "bottom" | "left" | "right";
    forceOpen?: boolean;
    disable?: boolean;
    divClassName?: string;
    divStyle?: CSSProperties;
    divOnClick?: (e: MouseEvent) => void;
    /** Show/hide delay in ms. Defaults to 300 (existing behavior). */
    delayMs?: number;
}

function TooltipInner(props: TooltipProps): JSX.Element {
    const placement: Placement = props.placement ?? "top";
    const forceOpen = () => props.forceOpen ?? false;
    const delayMs = () => props.delayMs ?? 300;

    const [isOpen, setIsOpen] = createSignal(forceOpen());
    const [isVisible, setIsVisible] = createSignal(false);
    const [floatingStyle, setFloatingStyle] = createSignal("position:absolute;left:0px;top:0px");

    let referenceEl: HTMLElement | null = null;
    let floatingEl: HTMLElement | null = null;
    let cleanupAutoUpdate: (() => void) | null = null;
    let showTimeout: ReturnType<typeof setTimeout> | null = null;
    let hideTimeout: ReturnType<typeof setTimeout> | null = null;

    const clearTimeouts = () => {
        if (showTimeout !== null) { clearTimeout(showTimeout); showTimeout = null; }
        if (hideTimeout !== null) { clearTimeout(hideTimeout); hideTimeout = null; }
    };

    const updatePosition = async () => {
        if (!referenceEl || !floatingEl) return;
        const pos = await computePosition(referenceEl, floatingEl, {
            placement,
            middleware: [offset(10), flip(), shift({ padding: 12 })],
        });
        setFloatingStyle(`position:absolute;left:${pos.x}px;top:${pos.y}px`);
    };

    const registerFloating = (el: HTMLElement) => {
        floatingEl = el;
        // Defer autoUpdate to next frame so the Portal has time to insert the
        // floating element into the DOM before floating-ui traverses ancestors.
        requestAnimationFrame(() => {
            if (referenceEl instanceof Element && floatingEl instanceof Element) {
                cleanupAutoUpdate?.();
                cleanupAutoUpdate = autoUpdate(referenceEl, floatingEl, updatePosition);
            }
        });
    };

    // Raw pointer-over state, tracked unconditionally (even while
    // `disable` is true) so a `disable` flip while the cursor is already
    // stationary over the anchor can still react — see the effect below.
    const [isHovering, setIsHovering] = createSignal(false);

    const handleMouseEnter = () => {
        if (forceOpen()) return;
        setIsHovering(true);
    };

    const handleMouseLeave = () => {
        if (forceOpen()) return;
        setIsHovering(false);
    };

    // Single reactive driver for open/visible, keyed off isHovering,
    // props.disable, and forceOpen — NOT a per-event imperative call inside
    // the handlers above. This matters because `disable` can change without
    // any mouse event firing at all (e.g. ToolBlock's virtualized tool rows
    // are reused across status transitions rather than remounted, so a tool
    // going from running/auto-expanded, disable=true to completed/collapsed,
    // disable=false happens with the cursor already stationary over the
    // anchor — no fresh `mouseenter` will ever fire to trigger an
    // imperative show). Because this effect tracks `props.disable` and
    // `isHovering()` together, the disable flip alone re-runs it and opens
    // the tooltip if the cursor is already there.
    createEffect(() => {
        clearTimeouts();
        if (props.disable) {
            // Hard close, bypassing the fade-out delay below — this path
            // fires when the underlying content changed out from under an
            // open/opening tooltip (not a deliberate mouse-away), so a
            // graceful fade would leave a stale tooltip floating over
            // content it no longer describes for the delay's duration.
            setIsOpen(false);
            setIsVisible(false);
        } else if (forceOpen()) {
            setIsOpen(true);
            setIsVisible(true);
        } else if (isHovering()) {
            setIsOpen(true);
            showTimeout = setTimeout(() => { setIsVisible(true); }, delayMs());
        } else {
            setIsVisible(false);
            hideTimeout = setTimeout(() => { setIsOpen(false); }, delayMs());
        }
    });

    onCleanup(() => {
        clearTimeouts();
        cleanupAutoUpdate?.();
    });

    return (
        <>
            <div
                ref={(el) => { referenceEl = el; }}
                class={props.divClassName}
                style={props.divStyle as any}
                onClick={props.divOnClick}
                onMouseEnter={handleMouseEnter}
                onMouseLeave={handleMouseLeave}
            >
                {props.children}
            </div>
            <Show when={isOpen()}>
                <Portal>
                    <div
                        ref={registerFloating}
                        style={`${floatingStyle()};opacity:${isVisible() ? 1 : 0};transition:opacity 200ms ease`}
                        class={cn(
                            "bg-gray-800 border border-border rounded-md px-2 py-1 text-xs text-foreground shadow-xl z-50"
                        )}
                        data-pane-overlay
                    >
                        {props.content}
                    </div>
                </Portal>
            </Show>
        </>
    );
}

export function Tooltip(props: TooltipProps): JSX.Element {
    return (
        <TooltipInner
            children={props.children}
            content={props.content}
            placement={props.placement}
            forceOpen={props.forceOpen}
            disable={props.disable}
            divClassName={props.divClassName}
            divStyle={props.divStyle}
            divOnClick={props.divOnClick}
            delayMs={props.delayMs}
        />
    );
}
