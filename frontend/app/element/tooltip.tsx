// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { AnchoredPopover } from "@/app/element/anchored-popover";
import { cn } from "@/util/util";
import { JSX, Show } from "solid-js";
import { createTooltip, type TooltipProps } from "./tooltip-state";

function TooltipInner(props: TooltipProps): JSX.Element {
    const t = createTooltip(props);

    return (
        <>
            <div
                ref={(el) => { t.setReference(el); }}
                class={props.divClassName}
                style={props.divStyle as any}
                onClick={props.divOnClick}
                onMouseEnter={t.handleMouseEnter}
                onMouseLeave={t.handleMouseLeave}
            >
                {props.children}
            </div>
            <Show when={t.isOpen()}>
                {/* AnchoredPopover places it, follows chrome zoom (the tooltip is the size of the bar it hangs
                    from) and cuts the hole in any native pane behind it; hidden until placed, so an `immediate`
                    tooltip never shows at the corner. */}
                <AnchoredPopover
                    anchor={t.reference()}
                    placement={t.placement}
                    gutter={10}
                    hideUntilPlaced
                    onShown={() => t.setIsPositioned(true)}
                    shellClass={props.passive ? "anchored-popover--tooltip anchored-popover--passive" : "anchored-popover--tooltip"}
                    class={cn("bg-modalbg border border-border rounded-md px-2 py-1 text-xs text-foreground shadow-xl")}
                    style={{
                        opacity: t.isVisible() && t.isPositioned() ? 1 : 0,
                        transition: t.immediate() ? "none" : "opacity var(--duration-instant) var(--ease-out)",
                    }}
                >
                    {props.content}
                </AnchoredPopover>
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
            immediate={props.immediate}
            passive={props.passive}
        />
    );
}
