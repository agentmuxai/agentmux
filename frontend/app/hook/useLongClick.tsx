// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, onCleanup } from "solid-js";

export const useLongClick = (
    getRef: () => HTMLElement | null | undefined,
    onClick?: (e: MouseEvent) => void,
    onLongClick?: (e: MouseEvent) => void,
    disabled = false,
    ms = 300
) => {
    let timerRef: ReturnType<typeof setTimeout> | null = null;
    let longClickTriggered = false;

    const startPress = (e: MouseEvent) => {
        if (onLongClick == null) {
            return;
        }
        longClickTriggered = false;
        timerRef = setTimeout(() => {
            longClickTriggered = true;
            onLongClick?.(e);
        }, ms);
    };

    const stopPress = () => {
        if (timerRef) {
            clearTimeout(timerRef);
            timerRef = null;
        }
    };

    const handleClick = (e: MouseEvent) => {
        if (longClickTriggered) {
            e.preventDefault();
            e.stopPropagation();
            return;
        }
        onClick?.(e);
    };

    createEffect(() => {
        const element = getRef();

        if (!element || disabled) return;

        element.addEventListener("mousedown", startPress);
        element.addEventListener("mouseup", stopPress);
        element.addEventListener("mouseleave", stopPress);
        element.addEventListener("click", handleClick);

        onCleanup(() => {
            element.removeEventListener("mousedown", startPress);
            element.removeEventListener("mouseup", stopPress);
            element.removeEventListener("mouseleave", stopPress);
            element.removeEventListener("click", handleClick);
        });
    });
};
