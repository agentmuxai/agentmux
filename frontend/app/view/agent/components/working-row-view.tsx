// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Working row's two forms: the worked line a turn leaves behind, and the live row with its spinner, line, readout and bar. */

import { Show, type Accessor, type JSX } from "solid-js";

import { AsciiSpinner } from "@/app/element/AsciiSpinner";
import { Tooltip } from "@/app/element/tooltip";
import { Button } from "@/app/element/ui/Button";
import type { RowMetrics } from "./working-row-metrics";
import type { RowState } from "./working-row-state";
import { WorkingRowOperation } from "./working-row-operation";

interface SummaryParts {
    workedSummary: Accessor<string | null>;
    workedSecondary: Accessor<string | null>;
    rightText: Accessor<string>;
    showCancelLogin: Accessor<boolean>;
}

export function WorkedLine(props: { summary: SummaryParts }): JSX.Element {
    const { workedSummary, workedSecondary } = props.summary;
    return (
        <Show when={workedSummary()}>
            <span class="agent-working-row agent-working-row--worked">
                <span class="agent-working-row-left">{workedSummary()}</span>
                <span class="agent-working-row-right">
                    <Show when={workedSecondary()}>
                        <span class="agent-working-row-secondary">{workedSecondary()}</span>
                    </Show>
                </span>
            </span>
        </Show>
    );
}

type Bar = NonNullable<ReturnType<RowMetrics["compactionBar"]>>;

function CompactionProgress(props: { bar: Accessor<Bar> }): JSX.Element {
    const bar = props.bar;
    return (
        <span
            class="agent-working-row-progress"
            classList={{ "is-over": bar().over }}
            role="progressbar"
            aria-label="Estimated compaction progress"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={bar().over ? undefined : Math.round(bar().fraction * 100)}
            aria-valuetext={
                bar().over
                    ? "Taking longer than the estimate"
                    : `About ${Math.round(bar().fraction * 100)} percent, estimated`
            }
        >
            <span class="agent-working-row-progress-fill" style={{ width: `${bar().fraction * 100}%` }} />
        </span>
    );
}

export function LiveLine(props: {
    s: RowState;
    settling: Accessor<boolean>;
    leftText: Accessor<string>;
    metrics: RowMetrics;
    summary: SummaryParts;
}): JSX.Element {
    const { s, metrics, summary } = props;
    const row = s.props;
    return (
        <span
            class="agent-working-row agent-working-row--loading"
            classList={{ "is-settling": !row.loading && props.settling() }}
        >
            <AsciiSpinner />
            <span class="agent-working-row-left">
                <span class="agent-working-row-primary">{props.leftText().slice(0, s.revealed())}</span>
            </span>
            <WorkingRowOperation row={row} />
            <Tooltip
                divClassName="agent-working-row-right"
                content={metrics.compactionTitle()}
                disable={!metrics.compactionTitle()}
            >
                {summary.rightText()}
                <Show when={summary.showCancelLogin()}>
                    <Button
                        variant="link"
                        class="agent-working-row-cancel-login"
                        onClick={() => row.onCancelLogin?.()}
                    >
                        Cancel
                    </Button>
                </Show>
            </Tooltip>
            <Show when={metrics.compactionBar()}>{(bar) => <CompactionProgress bar={bar} />}</Show>
        </span>
    );
}
