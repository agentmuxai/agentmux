// Copyright 2025, Command Line Inc.
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
import { createSignal, JSX, onCleanup, onMount, Show } from "solid-js";
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
}

function TooltipInner(props: Omit<TooltipProps, "disable">): JSX.Element {
    const placement: Placement = props.placement ?? "top";
    const forceOpen = () => props.forceOpen ?? false;

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

    const handleMouseEnter = () => {
        if (forceOpen()) return;
        clearTimeouts();
        setIsOpen(true);
        showTimeout = setTimeout(() => { setIsVisible(true); }, 300);
    };

    const handleMouseLeave = () => {
        if (forceOpen()) return;
        clearTimeouts();
        setIsVisible(false);
        hideTimeout = setTimeout(() => { setIsOpen(false); }, 300);
    };

    // React to forceOpen changes
    onMount(() => {
        if (forceOpen()) {
            setIsOpen(true);
            setIsVisible(true);
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
                    >
                        {props.content}
                    </div>
                </Portal>
            </Show>
        </>
    );
}

export function Tooltip(props: TooltipProps): JSX.Element {
    if (props.disable) {
        return (
            <div
                class={props.divClassName}
                style={props.divStyle as any}
                onClick={props.divOnClick}
            >
                {props.children}
            </div>
        );
    }

    return (
        <TooltipInner
            children={props.children}
            content={props.content}
            placement={props.placement}
            forceOpen={props.forceOpen}
            divClassName={props.divClassName}
            divStyle={props.divStyle}
            divOnClick={props.divOnClick}
        />
    );
}
