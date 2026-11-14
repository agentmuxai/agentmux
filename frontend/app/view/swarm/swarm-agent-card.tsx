// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The two parts of an agent's Swarm card: its header row (select, chevron, name, marks, chip) and the lines under it. */

import { Show, type Accessor, type JSX } from "solid-js";
import { UnseenTurnsDot } from "@/app/block/UnseenTurnsDot";
import { Tag } from "@/app/element/ui/Tag";
import { tip } from "@/app/element/tip";
import { HeldTaskChip } from "@/app/view/kanban/kanban-held";
import { formatCompactNumber } from "@/util/format-count";
import { awaitsBrowserAnswer, formatLineAge, isAwaitingUser, isMutedLine, lineAt, swarmLineTooltip, type SwarmLine } from "@/app/store/swarm-line";
import { unavailableLineText, unavailableTitle, type AgentUnavailable } from "@/app/store/agent-unavailable";
import type { AgentDisplayStatus } from "./swarm-display-status";
import { Highlight } from "./swarm-filter-bar";
import { highlightWords } from "./swarm-filter-state";
import { lastActiveTitle } from "./swarm-last-active";
import type { LongRunningToolRow } from "./swarm-longrunning";
import type { AgentTreeNode, SwarmViewModel } from "./swarm-model";
import { LOCAL_PROVIDER_UNKNOWN, SwarmProviderIcon } from "./swarm-provider-icon";
import { SpendChip } from "@/app/view/agent/components/SpendChip";
import { BuildSlotNote, LongToolNote, RowStateNotes, UnavailableChip } from "./swarm-row-notes";
import { RegistrationBadge, type RegistrationFlag } from "./swarm-row-registration";
import { SwarmSeatBadge } from "./swarm-seats";
import { AgentStatusChip } from "./swarm-status-chip";
import { setSupervisorMode, SupervisorChip } from "./swarm-supervisor";

const fmtCtx = formatCompactNumber;

/** The row's line, with how old it is once the agent's own line has gone stale (it counts on with the row's tick). */
function LineText(props: { line: SwarmLine; words: readonly string[]; now: number }): JSX.Element {
    return (
        <>
            <Highlight text={props.line.text} words={props.words} />
            <Show when={lineAt(props.line, props.now).stale && props.line.setAt}>
                {(setAt) => <span class="swarm-activity-age"> · {formatLineAge(props.now - setAt())}</span>}
            </Show>
        </>
    );
}

export type AgentCardHeadProps = {
    node: Accessor<AgentTreeNode>;
    model: SwarmViewModel;
    blockMeta: Accessor<Block["meta"] | undefined>;
    collapsed: Accessor<boolean>;
    hasChildren: Accessor<boolean>;
    totalRows: Accessor<number>;
    isParked: () => boolean;
    lastActive: Accessor<number | null>;
    lastActiveText: Accessor<string>;
    longRunningRows: Accessor<LongRunningToolRow[]>;
    registrationFlag: Accessor<RegistrationFlag | undefined>;
    unavailable: Accessor<AgentUnavailable | null>;
    displayStatus: Accessor<AgentDisplayStatus>;
    rowNow: () => number;
    /** The fleet-selection checkbox, drawn for a block that has an id. */
    selectBox: JSX.Element;
};

/** The header row of the card. */
export function AgentCardHead(props: AgentCardHeadProps): JSX.Element {
    const { node, model, blockMeta, collapsed, hasChildren, totalRows, isParked, lastActive, lastActiveText, longRunningRows, rowNow } = props;
    return (
        <div class="swarm-agent-row">
            {/* Fleet control selection (SPEC_MULTI_AGENT_FLEET_CONTROL_
                2026_08_20.md) — only a block with a real blockId can be
                a bulk-action target. stopPropagation so checking it
                doesn't also focus the card's own agent pane. */}
            <Show when={node().blockId} fallback={<span class="swarm-agent-select-spacer" />}>
                {props.selectBox}
            </Show>
            {/* Chevron only when there's a subtree to collapse; a
                fixed-width spacer otherwise so labels stay aligned
                across rows with and without children. Its own click
                handler now that the enclosing card's click focuses
                the agent's pane instead of toggling collapse
                (SPEC_SWARM_ROW_AGENT_COLOR_AND_SELECT_TO_FOCUS_2026_09_25.md
                §2.3) — stopPropagation so collapsing doesn't also
                move focus. */}
            <Show
                when={hasChildren()}
                fallback={<span class="swarm-agent-expand-spacer" />}
            >
                <i
                    class={`fa-solid fa-${collapsed() ? "chevron-right" : "chevron-down"} swarm-agent-expand-icon`}
                    onClick={(e) => {
                        e.stopPropagation();
                        if (node().blockId) model.toggleAgentCollapsed(node().blockId);
                    }}
                />
            </Show>
            <span class="swarm-agent-icon">
                <SwarmProviderIcon provider={node().agentProvider} unknownTitle={LOCAL_PROVIDER_UNKNOWN} />
            </span>
            <span class="swarm-agent-label">
                <Highlight text={node().agentName} words={highlightWords(model, "name")} />
            </span>
            <SwarmSeatBadge meta={blockMeta()} />
            {/* Running with no pane (closing its pane, tab or window parked it): the same
                mark My Agents puts on its row; a click on the row opens its pane. */}
            <Show when={isParked()}>
                <Tag tone="ok" title="Running with no pane. Click to open.">
                    No pane
                </Tag>
            </Show>
            {/* Turns this agent didn't get from you that finished
                unseen: the same mark as its pane header (UnseenTurnsDot). */}
            <Show when={node().blockId}>{(blockId) => <UnseenTurnsDot blockId={blockId()} />}</Show>
            <Show when={collapsed() && hasChildren()}>
                <span class="swarm-agent-collapsed-count">{totalRows()}</span>
            </Show>
            <Show when={node().contextTokens != null}>
                <span class="swarm-ctx-size">{fmtCtx(node().contextTokens!)}</span>
            </Show>
            <Show when={lastActiveText()}>
                <span class="swarm-last-active" {...tip(lastActiveTitle(lastActive()))}>
                    {lastActiveText()}
                </span>
            </Show>
            <LongToolNote rows={longRunningRows} live={() => node().agentStatus === "running"} now={rowNow} />
            <BuildSlotNote blockId={node().blockId} now={rowNow} />
            <RowStateNotes meta={blockMeta()} />
            <RegistrationBadge flag={props.registrationFlag} />
            <Show
                when={props.unavailable()}
                fallback={
                    <AgentStatusChip
                        status={props.displayStatus()}
                        awaitingUser={isAwaitingUser(blockMeta(), node().agentStatus)}
                        browserRequest={awaitsBrowserAnswer(blockMeta(), node().agentStatus)}
                        blockId={node().blockId}
                    />
                }
            >
                {(u) => <UnavailableChip unavailable={u()} now={rowNow} />}
            </Show>
            <Show when={node().blockId}>
                {(blockId) => (
                    <SupervisorChip
                        meta={blockMeta()}
                        live={node().agentStatus === "running"}
                        onSet={(mode) => setSupervisorMode(blockId(), mode)}
                    />
                )}
            </Show>
        </div>
    );
}

export type AgentCardLinesProps = {
    node: Accessor<AgentTreeNode>;
    model: SwarmViewModel;
    unavailable: Accessor<AgentUnavailable | null>;
    summaryFlash: Accessor<boolean>;
    blockMeta: Accessor<Block["meta"] | undefined>;
    rowNow: () => number;
};

/** The lines under the header: the summary, the Kanban task the agent is on, and the call in flight. */
export function AgentCardLines(props: AgentCardLinesProps): JSX.Element {
    const { node, unavailable, rowNow } = props;
    return (
        <>
            {/* Never empty: the generated title, or a fallback shown muted
                with a tooltip saying it is one (store/swarm-line.ts). */}
            <div
                classList={{
                    "swarm-activity-summary": true,
                    "swarm-activity-summary--flash": props.summaryFlash(),
                    "swarm-activity-summary--fallback": isMutedLine(node().line, rowNow()) || unavailable() !== null,
                }}
                {...tip(unavailable() ? unavailableTitle(unavailable()!) : swarmLineTooltip(node().line, rowNow()))}
            >
                {/* An agent that can't start a turn says so ahead of any title. Read from the
                    row's tick, like the chip, so the words go when the reset passes. */}
                {unavailable() ? (
                    unavailableLineText(unavailable()!, rowNow())
                ) : (
                    <LineText line={node().line} words={highlightWords(props.model, "work")} now={rowNow()} />
                )}
            </div>
            {/* The Kanban task this agent is on, which opens it on the Kanban pane (SPEC_WARDEN_KANBAN_2026_10_20.md §6.14). */}
            <Show when={node().blockId}>{(blockId) => <HeldTaskChip blockId={blockId()} />}</Show>
            {/* What an agent on a model route has used today against its cap: on a line of its own, since the header has no room for it. */}
            <SpendChip meta={props.blockMeta()} now={rowNow} />
            {/* The literal call in flight, next to the Haiku paraphrase
                above it. Deliberately OUTSIDE the collapsed() guard: it is
                one line, it is the most perishable thing on the card, and
                a collapsed agent is exactly when you want to know what it
                is doing without expanding it. */}
            <Show when={node().currentTool}>
                <div class="swarm-current-tool">
                    <i class="fa-solid fa-bolt swarm-current-tool-icon" />
                    <span class="swarm-current-tool-name">{node().currentTool}</span>
                </div>
            </Show>
        </>
    );
}
