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
                const { svg } = await mermaidInstance.render(id, normalizedChart);
                if (!cancelled) {
                    setSvgContent(svg);
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
                    <div style={{ color: "var(--error-color, #f44)", "margin-bottom": "8px" }}>
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
        <div style={{ color: "var(--error-color, #f44)", "margin-bottom": "8px" }}>Failed to render diagram</div>
        <pre style={{ "white-space": "pre-wrap", opacity: 0.7, "font-size": "0.85em" }}>{chart}</pre>
    </div>
);

export { Mermaid, MermaidErrorFallback };
