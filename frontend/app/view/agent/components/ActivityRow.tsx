// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ActivityRow — one uniform row in the pinned activity dock. Kind-agnostic
 * chrome (sigil + title + elapsed + tail + stop); the expanded view dispatches
 * by kind (Phase 1: shell → streaming log).
 *
 * Spec: docs/specs/SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md (§4)
 */

import clsx from "clsx";
import { Show, type JSX } from "solid-js";
import { ActivitySummary, GroupRoster, ShellLog, SubagentLog } from "./activity-row-parts";
import { createActivityRow, type ActivityRowProps } from "./activity-row-state";

export const ActivityRow = (props: ActivityRowProps): JSX.Element => {
    const s = createActivityRow(props);

    return (
        <Show when={props.activity()}>
            {(a) => (
                <div
                    ref={s.setRowRef}
                    class={clsx("agent-activity-row", a().kind, a().status, {
                        expanded: props.expanded(),
                        "activity-flash-out": !s.entering() && props.leaving(),
                    })}
                >
                    <ActivitySummary s={s} a={a} props={props} />

                    <Show when={props.expanded() && (a().shell || a().tool)}>
                        <ShellLog s={s} a={a} props={props} />
                    </Show>

                    <Show when={props.expanded() && a().subagent}>
                        <SubagentLog s={s} a={a} props={props} />
                    </Show>

                    <Show when={props.expanded() && a().subagentGroup}>
                        <GroupRoster s={s} a={a} props={props} />
                    </Show>
                </div>
            )}
        </Show>
    );
};

ActivityRow.displayName = "ActivityRow";
