// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import type { JSX } from "solid-js";

import "./card.scss";

/**
 * What a status dot says. Each surface maps its own states onto these:
 * an account's validity, a remote's connection, an MCP server's reports
 * (docs/specs/SPEC_CONNECTORS_CARDS_2026_10_13.md §3.3).
 */
export type StatusTone = "ok" | "busy" | "warn" | "error" | "idle";

export interface StatusDotProps {
    tone: StatusTone;
    /** Spoken and shown as the tooltip: "Connected", "Expired", … */
    label: string;
    class?: string;
}

/** A small round status light. `idle` is a hollow ring. */
export function StatusDot(props: StatusDotProps): JSX.Element {
    return (
        <span
            class={clsx("ui-status-dot", `ui-status-dot--${props.tone}`, props.class)}
            role="img"
            aria-label={props.label}
            title={props.label}
        />
    );
}
