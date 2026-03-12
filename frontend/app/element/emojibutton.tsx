// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { cn, makeIconClass } from "@/util/util";
import { createEffect, createSignal, JSX, Show } from "solid-js";

export const EmojiButton = (props: {
    emoji?: string;
    icon?: string;
    isClicked: boolean;
    onClick: () => void;
    className?: string;
    suppressFlyUp?: boolean;
}): JSX.Element => {
    const [showFloating, setShowFloating] = createSignal(false);
    let prevClicked = false;

    // Track isClicked changes to trigger float-up animation
    createEffect(() => {
        const current = props.isClicked;
        if (current && !prevClicked && !props.suppressFlyUp) {
            setShowFloating(true);
            setTimeout(() => setShowFloating(false), 600);
        }
        prevClicked = current;
    });

    const content = () => props.icon ? <i class={makeIconClass(props.icon, false)} /> : props.emoji;

    return (
        <div class="relative inline-block">
            <button
                onClick={props.onClick}
                class={cn(
                    "px-2 py-1 rounded border cursor-pointer transition-colors",
                    props.isClicked
                        ? "bg-accent/20 border-accent text-accent"
                        : "bg-transparent border-border/50 text-foreground/70 hover:border-border",
                    props.className
                )}
            >
                {content()}
            </button>
            <Show when={showFloating()}>
                <span
                    class="absolute pointer-events-none animate-[float-up_0.6s_ease-out_forwards]"
                    style={{
                        left: "50%",
                        bottom: "100%",
                    }}
                >
                    {content()}
                </span>
            </Show>
        </div>
    );
};
