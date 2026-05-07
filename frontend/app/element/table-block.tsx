// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { writeText as clipboardWriteText } from "@/util/clipboard";
import { cn } from "@/util/util";
import { createSignal, JSX, Show } from "solid-js";

function extractCsv(table: HTMLTableElement): string {
    const rows: string[] = [];
    for (const row of Array.from(table.rows)) {
        const cells = Array.from(row.cells).map((cell) => {
            const text = (cell.innerText ?? "").replace(/\r?\n/g, " ").trim();
            return /[,"\n]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
        });
        rows.push(cells.join(","));
    }
    return rows.join("\n");
}

interface TableBlockProps {
    children?: JSX.Element;
    class?: string;
}

export function TableBlock(props: TableBlockProps): JSX.Element {
    let tableRef: HTMLTableElement | undefined;
    const [copied, setCopied] = createSignal(false);

    const handleCopy = async () => {
        if (!tableRef) return;
        const csv = extractCsv(tableRef);
        await clipboardWriteText(csv);
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
    };

    return (
        <div class="table-block group relative my-4 overflow-x-auto rounded-lg border border-border">
            <button
                class={cn(
                    "absolute right-2 top-2 z-10 rounded border border-border px-2 py-0.5",
                    "text-xs text-muted opacity-0 transition-opacity group-hover:opacity-100",
                    "bg-panel hover:bg-hover",
                    copied() && "text-success opacity-100"
                )}
                onClick={handleCopy}
                title="Copy as CSV"
            >
                <Show when={copied()} fallback="CSV">
                    ✓ copied
                </Show>
            </button>
            <table ref={tableRef} class={cn("w-full border-collapse", props.class)}>
                {props.children}
            </table>
        </div>
    );
}
