// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sysinfo's charts, loaded with the first one drawn rather than at startup:
 * uPlot is only needed once a Sysinfo pane is open
 * (SPEC_FRONTEND_BUNDLE_SPLITTING_2026_10_21.md §5). The pane around a chart
 * renders at once; the chart appears when its code arrives.
 *
 * Not Solid's `lazy()`: it keeps a failed load's promise, so a chunk that
 * failed once would fail for good, even after the pane's Reload. Here a
 * failed load shows in the chart's place with Try again, which imports anew.
 */

import { Button } from "@/app/element/ui";
import type { ComponentProps, JSX } from "solid-js";
import { createSignal, Show } from "solid-js";
import { Dynamic } from "solid-js/web";

type PlotModule = typeof import("./sysinfo-plot");

// Shared by every chart, and reactive: when any chart's Try again succeeds,
// every chart waiting on the module draws.
const [loaded, setLoaded] = createSignal<PlotModule>();
const [failure, setFailure] = createSignal<string>();
let loading: Promise<PlotModule> | undefined;

function loadPlot(): void {
    if (loaded() || loading) return;
    setFailure(undefined);
    loading = import("./sysinfo-plot").then(
        (m) => {
            setLoaded(() => m);
            return m;
        },
        (e) => {
            loading = undefined;
            console.error("[sysinfo] the chart code failed to load:", e);
            setFailure(e instanceof Error ? e.message : String(e));
            throw e;
        }
    );
    loading.catch(() => {});
}

/** `pick`'s component from the plot module, once loaded. */
function lazyChart<P extends object>(pick: (m: PlotModule) => (props: P) => JSX.Element) {
    return (props: P): JSX.Element => {
        loadPlot();
        return (
            <Show
                when={loaded()}
                fallback={
                    <Show when={failure()}>
                        <div role="alert">
                            Chart failed to load: {failure()}{" "}
                            <Button density="compact" onClick={loadPlot}>
                                Try again
                            </Button>
                        </div>
                    </Show>
                }
            >
                {(m) => <Dynamic component={pick(m())} {...props} />}
            </Show>
        );
    };
}

export const TimeChart = lazyChart<ComponentProps<PlotModule["TimeChart"]>>((m) => m.TimeChart);
export const SingleLinePlot = lazyChart<ComponentProps<PlotModule["SingleLinePlot"]>>((m) => m.SingleLinePlot);
