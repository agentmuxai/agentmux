// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { JSX } from "solid-js";

import { Tag } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { SeatSummary } from "@/types/rpc/SeatSummary";
import { seatBadgeText, seatHue } from "./seat-format";

/**
 * A seat as a coloured tag: "Leader · product", in the role's own colour, on every surface that shows one (the
 * Swarm, My Agents, the Atlas), so a seat reads the same wherever it is. The words are what `seat-format.ts` says; the
 * colour only reinforces them, and a screen reader hears "Seat Leader product". A role this build doesn't know shows
 * as written, in the neutral tone.
 * docs/specs/SPEC_SWARM_MODEL_AND_SEAT_TAGS_2026_11_20.md §5.
 */
export function SeatTag(props: {
    seat: Pick<SeatSummary, "role" | "qualifier">;
    class?: string;
    size?: "sm" | "xs";
    /** A native tooltip (a seat's tooltip line is longer than its words). */
    title?: string;
}): JSX.Element {
    const hue = () => seatHue(props.seat.role) ?? undefined;
    return (
        <Tag hue={hue()} tone={hue() ? undefined : "neutral"} size={props.size} class={props.class} title={props.title}>
            <span class="ui-tag-sr">{t("swarm.tag.seat")} </span>
            {seatBadgeText(props.seat)}
        </Tag>
    );
}
