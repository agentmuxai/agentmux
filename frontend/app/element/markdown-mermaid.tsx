// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, onCleanup, onMount, Show } from "solid-js";

let mermaidInitialized = false;
let mermaidInstance: any = null;
let mermaidRenderCount = 0;

const initializeMermaid = async () => {
    if (!mermaidInitialized) {
        const mermaid = await import("mermaid");
        mermaidInstance = mermaid.default;
        mermaidInstance.initialize({
            startOnLoad: false,
            theme: "dark",
            securityLevel: "strict",
        });
        mermaidInitialized = true;
    }
};

/**
 * Mermaid renders into a scratch container it appends to <body> (`#d<id>`, with the diagram's own `#<id>` in it).
 * It takes the container away when the render succeeds, but not when the diagram fails to parse: the error SVG stays
 * in <body>, which is a flex row, so each failed diagram becomes a column beside the app and shrinks it. Called after
 * every render, so success and failure both leave nothing behind. The diagram shown in the pane is a copy of the SVG
 * text, so this never touches it.
 */
export function removeMermaidScratch(id: string, doc: Document = document): void {
    doc.getElementById(`d${id}`)?.remove();
    const stray = doc.getElementById(id);
    if (stray && stray.parentElement === doc.body) stray.remove();
}

const Mermaid = ({ chart }: { chart: string }) => {
    let ref!: HTMLDivElement;
    const [svgContent, setSvgContent] = createSignal<string | null>(null);
    const [error, setError] = createSignal<string | null>(null);

    onMount(() => {
        let cancelled = false;
        const renderMermaid = async () => {
            try {
                setError(null);
                setSvgContent(null);

                await initializeMermaid();
                if (cancelled || !mermaidInstance) {
                    return;
                }

                // Normalize the chart text
                const normalizedChart = chart
                    .replace(/<br\s*\/?>/gi, "\n")
                    .replace(/\r\n?/g, "\n")
                    .replace(/\n+$/, "");

                const id = `mermaid-${++mermaidRenderCount}`;
                try {
                    const { svg } = await mermaidInstance.render(id, normalizedChart);
                    if (!cancelled) {
                        setSvgContent(svg);
                    }
                } finally {
                    removeMermaidScratch(id);
                }
            } catch (err: any) {
                console.error("Error rendering mermaid diagram:", err);
                if (!cancelled) {
                    setError(err.message || String(err));
                }
            }
        };

        renderMermaid();
        onCleanup(() => {
            cancelled = true;
        });
    });

    return (
        <Show
            when={!error()}
            fallback={
                <div class="mermaid error">
                    <div style={{ color: "var(--error-text-color)", "margin-bottom": "8px" }}>
                        Failed to render diagram
                    </div>
                    <pre style={{ "white-space": "pre-wrap", opacity: 0.7, "font-size": "0.85em" }}>{chart}</pre>
                </div>
            }
        >
            <Show
                when={svgContent()}
                fallback={<div class="mermaid">Loading diagram...</div>}
            >
                <div class="mermaid" ref={ref} innerHTML={svgContent()} />
            </Show>
        </Show>
    );
};

const MermaidErrorFallback = ({ error, chart }: { error?: Error; chart: string }) => (
    <div class="mermaid error">
        <div style={{ color: "var(--error-text-color)", "margin-bottom": "8px" }}>Failed to render diagram</div>
        <pre style={{ "white-space": "pre-wrap", opacity: 0.7, "font-size": "0.85em" }}>{chart}</pre>
    </div>
);

export { Mermaid, MermaidErrorFallback };
