// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * RecordTable — render a top-level array of flat records as a table instead of a
 * JSON blob. Registered by shape *below* the coarse-kind built-ins, so it only
 * improves the otherwise-JSON unknown-tool path.
 *
 * See SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md §5.3 (Phase 3).
 */

import { For, Show, type JSX } from "solid-js";
import type { ToolNode } from "../../types";
import { CompactResult } from "../CompactResult";
import { OutputHiddenMarker } from "../OutputHiddenMarker";
import { byShape, registerToolRenderer } from "./registry";
import { extractRecords, looksLikeRecords, cellText } from "./record-table";

export function RecordTable(props: { node: ToolNode }): JSX.Element {
    const data = extractRecords(props.node.result);
    return (
        <Show
            when={data}
            fallback={
                <CompactResult
                    tool={props.node.tool}
                    params={props.node.params as any}
                    result={props.node.result}
                />
            }
        >
            <div class="agent-tool-record-table">
                <table>
                    <thead>
                        <tr>
                            <For each={data!.columns}>{(c) => <th>{c}</th>}</For>
                        </tr>
                    </thead>
                    <tbody>
                        <For each={data!.rows}>
                            {(row) => (
                                <tr>
                                    <For each={data!.columns}>{(c) => <td>{cellText(row[c])}</td>}</For>
                                </tr>
                            )}
                        </For>
                    </tbody>
                </table>
                <Show when={data!.truncatedRows > 0}>
                    <OutputHiddenMarker hidden={data!.truncatedRows} noun="line" from="head" />
                </Show>
            </div>
        </Show>
    );
}

RecordTable.displayName = "RecordTable";

// Register by shape at priority -1: above the JSON catch-all (-Infinity) but
// below the coarse-kind built-ins (0) and name-matched rich renderers (10), so a
// record list from an unknown tool becomes a table while known tools are
// untouched.
registerToolRenderer({
    priority: -1,
    label: "shape:record-table",
    match: byShape(looksLikeRecords),
    render: (node) => <RecordTable node={node} />,
});
