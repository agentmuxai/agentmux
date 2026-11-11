// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { AnchoredPopover } from "@/app/element/anchored-popover";
import { cn } from "@/util/util";
import type { Placement } from "@floating-ui/dom";
import { createEffect, createSignal, JSX, onCleanup, Show } from "solid-js";
import type { Properties as CSSProperties } from "csstype";
import { isPrimaryButtonDown, onPrimaryButtonRelease } from "@/app/util/pointer-drag-state";
import { claimOpen, forgetOpen, HIDE_DELAY_MS, releaseOpen, SHOW_DELAY_MS, isWarm, type OpenTip } from "./tooltip-timing";

interface TooltipProps {
    children?: JSX.Element;
    content?: JSX.Element;
    placement?: "top" | "bottom" | "left" | "right";
    forceOpen?: boolean;
    disable?: boolean;
    divClassName?: string;
    divStyle?: CSSProperties;
    divOnClick?: (e: MouseEvent) => void;
    /**
     * Show/hide delay in ms. Defaults to near-instant: it shows 40 ms after the pointer rests on
     * the anchor (at once when another tooltip has just closed, so sweeping along a toolbar reads
     * continuously), and lingers 100 ms after it leaves so a gap between two items does not flicker.
     */
    delayMs?: number;
    /**
     * No delay and no fade: the panel appears the moment the pointer enters (as soon
     * as it has been positioned, at most one frame, so it never flashes at the
     * top-left corner) and is gone the moment the pointer leaves. For tooltips a user
     * reads at a glance, such as an agent's summary on its pane tab. Overrides
     * `delayMs`.
     */
    immediate?: boolean;
    /**
     * A status the pointer only reads: the panel takes no pointer events, so it can never sit under the
     * pointer and end the hover that opened it (the anchor's mouseleave, a close, a reopen, every frame).
     * Its text can't be selected either. Not for a tooltip with a link or a control in it.
     */
    passive?: boolean;
}

function TooltipInner(props: TooltipProps): JSX.Element {
    const placement: Placement = props.placement ?? "top";
    const forceOpen = () => props.forceOpen ?? false;
    const immediate = () => props.immediate ?? false;
    const showDelayMs = () => (immediate() ? 0 : (props.delayMs ?? (isWarm(self) ? 0 : SHOW_DELAY_MS)));
    const hideDelayMs = () => (immediate() ? 0 : (props.delayMs ?? HIDE_DELAY_MS));

    const [isOpen, setIsOpen] = createSignal(forceOpen());
    const [isVisible, setIsVisible] = createSignal(false);
    // Whether the panel has been placed at least once since it opened. `immediate`
    // keeps it invisible until then, so it cannot show at the unpositioned corner.
    const [isPositioned, setIsPositioned] = createSignal(false);

    let referenceEl: HTMLElement | null = null;
    let showTimeout: ReturnType<typeof setTimeout> | null = null;
    let hideTimeout: ReturnType<typeof setTimeout> | null = null;

    const clearTimeouts = () => {
        if (showTimeout !== null) { clearTimeout(showTimeout); showTimeout = null; }
        if (hideTimeout !== null) { clearTimeout(hideTimeout); hideTimeout = null; }
    };

    // Closes this tooltip now, without the lingering grace.
    const self: OpenTip = {
        close: () => {
            clearTimeouts();
            releaseOpen(self);
            setIsOpen(false);
            setIsVisible(false);
            setIsPositioned(false);
        },
    };

    // Raw pointer-over state, tracked unconditionally (even while
    // `disable` is true) so a `disable` flip while the cursor is already
    // stationary over the anchor can still react — see the effect below.
    const [isHovering, setIsHovering] = createSignal(false);

    // Entry (mount) no-ops while the mouse's primary button is held — a
    // text-selection drag elsewhere on the page still delivers mouseenter
    // to whatever anchor the cursor sweeps over, and letting it through
    // here would mount this Portal-rendered panel under the cursor
    // mid-drag, which was intermittently breaking the browser's native
    // selection-extend hit-testing — the same mechanism `useNodePeek`'s
    // `PeekOverlay` had. See pointer-drag-state.ts and
    // docs/plans/PLAN_AGENT_PANE_TEXT_SELECTION_DRAG_FLICKER_2026_09_20.md.
    // Leave (unmount) is deliberately NOT gated — see that hook's own
    // comment (reagentx P1 on PR #3470): gating it too left a tooltip
    // stuck open indefinitely whenever mouseleave fired mid-drag but the
    // button was released elsewhere, since no further mouseenter/
    // mouseleave would ever fire for an anchor the cursor had already left.
    // Hover is tracked even while `forceOpen` holds the tooltip open, so the pointer state is true when
    // the hold ends (a chip pinned by focus that the pointer has since left must close on blur).
    const handleMouseEnter = () => {
        if (isPrimaryButtonDown()) return;
        setIsHovering(true);
    };

    // The gate above drops the mouseenter that arrives mid-drag. If the drag
    // then ends with the cursor still on this anchor, no further mouseenter
    // ever fires for it and the tooltip stays shut until the user leaves and
    // comes back. Re-check the real hover state when the button is released
    // (reagentx P2 on PR #3470).
    onCleanup(
        onPrimaryButtonRelease(() => {
            if (forceOpen()) return;
            if (referenceEl?.matches(":hover")) setIsHovering(true);
        }),
    );

    const handleMouseLeave = () => {
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
            setIsPositioned(false);
            releaseOpen(self);
        } else if (forceOpen()) {
            setIsOpen(true);
            setIsVisible(true);
        } else if (isHovering()) {
            claimOpen(self, !immediate());
            setIsOpen(true);
            const delay = showDelayMs();
            if (immediate() || delay === 0) setIsVisible(true);
            else showTimeout = setTimeout(() => { setIsVisible(true); }, delay);
        } else {
            setIsVisible(false);
            if (immediate()) {
                // Gone the moment the pointer leaves: no grace period, no fade.
                setIsOpen(false);
                setIsPositioned(false);
                forgetOpen(self);
            } else {
                hideTimeout = setTimeout(() => { setIsOpen(false); setIsPositioned(false); releaseOpen(self); }, hideDelayMs());
            }
        }
    });

    onCleanup(() => {
        clearTimeouts();
        forgetOpen(self);
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
                {/* AnchoredPopover places it, follows chrome zoom (the tooltip is the size of the bar it hangs
                    from) and cuts the hole in any native pane behind it; hidden until placed, so an `immediate`
                    tooltip never shows at the corner. */}
                <AnchoredPopover
                    anchor={referenceEl}
                    placement={placement}
                    gutter={10}
                    hideUntilPlaced
                    onShown={() => setIsPositioned(true)}
                    shellClass={props.passive ? "anchored-popover--tooltip anchored-popover--passive" : "anchored-popover--tooltip"}
                    class={cn("bg-modalbg border border-border rounded-md px-2 py-1 text-xs text-foreground shadow-xl")}
                    style={{
                        opacity: isVisible() && isPositioned() ? 1 : 0,
                        transition: immediate() ? "none" : "opacity var(--duration-instant) var(--ease-out)",
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
