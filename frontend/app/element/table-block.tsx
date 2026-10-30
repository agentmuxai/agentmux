// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { writeText as clipboardWriteText } from "@/util/clipboard";
import { cn } from "@/util/util";
import { Button } from "@/element/ui";
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
        try {
            const csv = extractCsv(tableRef);
            await clipboardWriteText(csv);
            setCopied(true);
            setTimeout(() => setCopied(false), 1500);
        } catch (e) {
            console.warn("TableBlock: clipboard write failed", e);
        }
    };

    return (
        <div class="table-block group relative my-4 overflow-x-auto rounded-lg border border-border">
            <Button
                density="compact"
                class={cn(
                    "absolute right-2 top-2 z-10 transition-opacity",
                    copied() ? "opacity-100" : "opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
                )}
                style={{ "--ui-bg": "var(--color-panel)", ...(copied() ? { "--ui-fg": "var(--success-text-color)" } : {}) }}
                onClick={handleCopy}
                title="Copy as CSV"
            >
                <Show when={copied()} fallback="CSV">
                    ✓ copied
                </Show>
            </Button>
            <table ref={tableRef} class={cn("w-full border-collapse", props.class)}>
                {props.children}
            </table>
        </div>
    );
}
