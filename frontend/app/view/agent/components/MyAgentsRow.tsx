// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// One tile of the My agents list: the agent's logo, name, state and badges, its account, preview and times,
// and the chevron that opens its panel. Split out of MyAgentsList.tsx (#13044 S6); the panel under it is
// MyAgentsRowPanel.tsx.

import { Tooltip } from "@/app/element/tooltip";
import { Button, IconButton, Pill } from "@/app/element/ui";
import { hasProcess } from "@/app/store/agent-runtime";
import { DualProviderLogo } from "@/element/DualProviderLogo";
import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";
import { formatTimeAgo } from "@/util/format-time";
import clsx from "clsx";
import { Show, type Accessor, type JSX } from "solid-js";
import { resolveEffectiveVendor } from "../providers/catalog";
import { seatBadgeText, seatReportsLine, seatTooltipLine } from "../seat-format";
import { AgentRuntimeChip } from "./AgentRuntimeChip";
import { noPreviewText } from "./my-agents-list-text";
import type { MyAgentsListProps } from "./MyAgentsList";
import { RuntimeBadge } from "./RuntimeBadge";

export interface MyAgentsRowProps {
    row: RecentSessionRow;
    list: MyAgentsListProps;
    now: Accessor<number>;
    isExpanded: () => boolean;
    onToggle: () => void;
    tileRef: (el: HTMLLIElement) => void;
}

export const MyAgentsRow = (props: MyAgentsRowProps): JSX.Element => {
    const row = props.row;
    const list = props.list;
    const isActive = () => (list.openDefinitions?.() ?? new Map()).has(row.definition_id);
    const noPane = () => {
        const runtime = list.runtimeOf?.(row.definition_id);
        return !!runtime && hasProcess(runtime.state) && !isActive();
    };

    // The tile is the `<li>` (the grid item): it owns the border,
    // background and hover, so the chevron and the panels are placed
    // against the box the user sees. See
    // docs/reports/REPORT_MY_AGENTS_TILE_ACTIONS_ALIGNMENT_2026_10_05.md.
    const openWhere = () => list.openLocations?.().get(row.definition_id)?.label ?? "open in another pane";

    return (
        <li
            ref={props.tileRef}
            class="agent-recent-sessions-row"
            classList={{
                "is-expanded": props.isExpanded(),
                "agent-recent-sessions-row--active": isActive(),
                "agent-recent-sessions-row--no-pane": noPane(),
            }}
            data-definition-id={row.definition_id}
        >
            <DualProviderLogo
                harness={row.provider}
                vendor={resolveEffectiveVendor(row.provider, row.model_vendor_base_url)}
                size={40}
                class="agent-recent-sessions-icon"
            />
            <span class="agent-recent-sessions-body">
                <span class="agent-recent-sessions-line1">
                    {/* The one control that opens the agent. Its ::after
                                                covers the whole tile (the "stretched button"
                                                pattern), so a click anywhere on the tile opens it
                                                while the text stays plain text and the chevron
                                                stays a separate control: nested buttons are not
                                                valid HTML. */}
                    <Button
                        variant="row"
                        density="compact"
                        class="agent-recent-sessions-entry"
                        onClick={() => props.onToggle()}
                        aria-expanded={props.isExpanded()}
                        aria-label={row.instance_name || row.definition_name}
                        data-testid="agent-my-agents-entry"
                    >
                        <span class="agent-recent-sessions-name">{row.instance_name || row.definition_name}</span>
                    </Button>
                    <Show when={list.runtimeOf?.(row.definition_id)}>
                        {(runtime) => <AgentRuntimeChip state={runtime().state} detail={runtime().detail} />}
                    </Show>
                    <Show when={noPane()}>
                        <span class="agent-no-pane-tag" data-testid="agent-no-pane-tag">
                            no pane
                        </span>
                    </Show>
                    <Show when={isActive()}>
                        <Tooltip
                            content={openWhere().charAt(0).toUpperCase() + openWhere().slice(1)}
                            divClassName="agent-active-badge-anchor"
                        >
                            <span class="agent-active-badge" aria-label="Active" />
                        </Tooltip>
                    </Show>
                    <Show when={row.agent_type === "host" || row.agent_type === "container"}>
                        {/* size="tag" — same NATIVE/CONTAINER wording + white/
                                                    yellow styling as AgentComposerStrip's runtime
                                                    tag, so the badge reads as one consistent
                                                    vocabulary instead of two ("Container" here vs.
                                                    "CONTAINER" in the pane the row launches into). */}
                        <RuntimeBadge runtime={row.agent_type} size="tag" />
                    </Show>
                    <Show when={row.seat}>
                        {(seat) => (
                            <Pill class="agent-seat-badge" title={seatTooltipLine(seat())}>
                                {seatBadgeText(seat())}
                            </Pill>
                        )}
                    </Show>
                </span>
                {/* Own line, not squeezed onto line1 via flex:1 next to
                                            the name — the account is as important as the name
                                            itself for a user managing multiple accounts, and
                                            needs reliable space rather than competing ellipsis
                                            with it. See
                                            docs/reports/REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md §2. */}
                {/* "No auth", or the bound account; empty (so no line) when
                                            the backend could not tell
                                            (SPEC_MY_AGENTS_TILES_AUTH_AND_HISTORY_2026_10_03.md §5.2). */}
                <Show when={row.identity_name}>
                    <span class="agent-recent-sessions-account">{row.identity_name}</span>
                </Show>
                <Show when={row.seat}>
                    {(seat) => (
                        <span class="agent-recent-sessions-seat" data-testid="agent-my-agents-seat-line">
                            {seatReportsLine(seat())}
                        </span>
                    )}
                </Show>
                <Show
                    when={row.preview}
                    fallback={
                        <span class="agent-recent-sessions-preview agent-recent-sessions-preview--empty">
                            {noPreviewText(row)}
                        </span>
                    }
                >
                    <span class="agent-recent-sessions-preview">{row.preview}</span>
                </Show>
                <Show when={row.node_count > 0}>
                    <span class="agent-recent-sessions-line3">
                        <span class="agent-recent-sessions-nodes">
                            {row.node_count} message
                            {row.node_count === 1 ? "" : "s"}
                        </span>
                    </span>
                </Show>
            </span>
            {/* Footer row: the timestamps and the chevron share one row at
                                        the bottom of the tile. Most-relevant-first, not
                                        chronological (Created → Launch → Active): a picker exists to
                                        answer "what did I last touch," which is Last Active (or Last
                                        Launch if never active) — not when the agent was originally
                                        created, which is the least actionable fact on the row. See
                                        docs/reports/REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md §2. */}
            <span class="agent-recent-sessions-timestamps">
                <Show when={row.has_snapshot && row.started_at > 0 && row.last_active_at > row.started_at}>
                    <span class="agent-recent-sessions-ts">
                        <span class="agent-recent-sessions-ts-label">Last Active</span>
                        <span class="agent-recent-sessions-ts-value">
                            {formatTimeAgo(row.last_active_at, props.now())}
                        </span>
                    </span>
                </Show>
                <Show when={row.started_at > 0}>
                    <span class="agent-recent-sessions-ts">
                        <span class="agent-recent-sessions-ts-label">Last Launch</span>
                        <span class="agent-recent-sessions-ts-value">{formatTimeAgo(row.started_at, props.now())}</span>
                    </span>
                </Show>
                <Show when={row.agent_created_at > 0}>
                    <span class="agent-recent-sessions-ts">
                        <span class="agent-recent-sessions-ts-label">Created</span>
                        <span class="agent-recent-sessions-ts-value">
                            {formatTimeAgo(row.agent_created_at, props.now())}
                        </span>
                    </span>
                </Show>
            </span>

            {/* Row actions menu toggle. A sibling of the entry button, not
                                        nested inside it (nested buttons are invalid HTML). See
                                        docs/specs/SPEC_AGENT_DELETE_2026_09_16.md §4.1. */}
            <IconButton
                icon="chevron-down"
                label={`Actions for ${row.instance_name || row.definition_name}`}
                tooltip={false}
                density="compact"
                class={clsx("agent-recent-sessions-menu-toggle", props.isExpanded() && "is-open")}
                aria-expanded={props.isExpanded()}
                onClick={(e) => {
                    e.stopPropagation();
                    props.onToggle();
                }}
                data-testid="agent-my-agents-menu-toggle"
            />
        </li>
    );
};
