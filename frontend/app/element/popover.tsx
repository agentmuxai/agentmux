// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { Button } from "@/element/button";
import {
    autoUpdate,
    computePosition,
    offset as offsetMiddleware,
    type Middleware,
    type OffsetOptions,
    type Placement,
} from "@floating-ui/dom";
import clsx from "clsx";
import {
    createContext,
    createSignal,
    JSX,
    onCleanup,
    onMount,
    Show,
    useContext,
} from "solid-js";
import { Portal } from "solid-js/web";

import "./popover.scss";

interface PopoverContextType {
    isOpen: () => boolean;
    togglePopover: () => void;
    closePopover: () => void;
    registerReference: (el: HTMLElement) => void;
    registerFloating: (el: HTMLElement) => void;
    floatingStyle: () => string;
}

const PopoverContext = createContext<PopoverContextType | null>(null);

interface PopoverProps {
    children?: JSX.Element;
    class?: string;
    className?: string; // legacy compat
    placement?: Placement;
    offset?: OffsetOptions;
    onDismiss?: () => void;
    middleware?: Middleware[];
}

const Popover = (props: PopoverProps): JSX.Element => {
    const placement = props.placement ?? "bottom-start";
    const offsetVal = props.offset ?? 3;

    const [isOpen, setIsOpen] = createSignal(false);
    const [floatingStyle, setFloatingStyle] = createSignal("position:absolute;left:0px;top:0px");

    let referenceEl: HTMLElement | null = null;
    let floatingEl: HTMLElement | null = null;
    let cleanupAutoUpdate: (() => void) | null = null;

    const middleware: Middleware[] = [...(props.middleware ?? []), offsetMiddleware(offsetVal as any)];

    const updatePosition = async () => {
        if (!referenceEl || !floatingEl) return;
        const pos = await computePosition(referenceEl, floatingEl, { placement, middleware });
        setFloatingStyle(`position:absolute;left:${pos.x}px;top:${pos.y}px`);
    };

    const openPopover = () => {
        setIsOpen(true);
        // updatePosition is called by autoUpdate once floatingEl is set
    };

    const closePopover = () => {
        setIsOpen(false);
        cleanupAutoUpdate?.();
        cleanupAutoUpdate = null;
        props.onDismiss?.();
    };

    const togglePopover = () => (isOpen() ? closePopover() : openPopover());

    const registerReference = (el: HTMLElement) => {
        referenceEl = el;
    };

    const registerFloating = (el: HTMLElement) => {
        floatingEl = el;
        requestAnimationFrame(() => {
            if (referenceEl instanceof Element && floatingEl instanceof Element) {
                cleanupAutoUpdate?.();
                cleanupAutoUpdate = autoUpdate(referenceEl, floatingEl, updatePosition);
            }
        });
    };

    const handleClickOutside = (e: MouseEvent) => {
        if (!isOpen()) return;
        const target = e.target as Node;
        if (referenceEl?.contains(target) || floatingEl?.contains(target)) return;
        closePopover();
    };

    onMount(() => {
        document.addEventListener("mousedown", handleClickOutside);
    });

    onCleanup(() => {
        document.removeEventListener("mousedown", handleClickOutside);
        cleanupAutoUpdate?.();
    });

    const ctx: PopoverContextType = {
        isOpen,
        togglePopover,
        closePopover,
        registerReference,
        registerFloating,
        floatingStyle,
    };

    return (
        <PopoverContext.Provider value={ctx}>
            <div class={clsx("popover", props.className)}>
                {props.children}
            </div>
        </PopoverContext.Provider>
    );
};

interface PopoverButtonProps extends JSX.ButtonHTMLAttributes<HTMLButtonElement> {
    isActive?: boolean;
    children?: JSX.Element;
    as?: string | ((props: any) => JSX.Element);
    className?: string;
}

const PopoverButton = (props: PopoverButtonProps): JSX.Element => {
    const ctx = useContext(PopoverContext);

    const handleClick = (e: MouseEvent) => {
        ctx?.togglePopover();
        if (props.onClick) (props.onClick as any)(e);
    };

    return (
        <Button
            ref={(el: HTMLElement) => ctx?.registerReference(el)}
            class={clsx("popover-button", props.className)}
            classList={{ "is-active": ctx?.isOpen() }}
            {...(props as any)}
            onClick={handleClick}
        >
            {props.children}
        </Button>
    );
};

interface PopoverContentProps extends JSX.HTMLAttributes<HTMLDivElement> {
    children?: JSX.Element;
    className?: string;
}

const PopoverContent = (props: PopoverContentProps): JSX.Element => {
    const ctx = useContext(PopoverContext);

    return (
        <Show when={ctx?.isOpen()}>
            <Portal>
                <div
                    ref={(el) => ctx?.registerFloating(el)}
                    class={clsx("popover-content", props.className)}
                    style={ctx?.floatingStyle()}
                    {...(props as any)}
                >
                    {props.children}
                </div>
            </Portal>
        </Show>
    );
};

export { Popover, PopoverButton, PopoverContent };
export type { PopoverButtonProps, PopoverContentProps };
