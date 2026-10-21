// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sysinfo's charts, loaded with the first one drawn rather than at startup:
 * uPlot is only needed once a Sysinfo pane is open
 * (SPEC_FRONTEND_BUNDLE_SPLITTING_2026_10_21.md §5). Each chart waits in its own
 * `Suspense`, so the pane around it renders at once and never shows the block's
 * loading spinner for it. A chunk that fails to load reaches the pane's error
 * boundary.
 */

import type { ComponentProps, JSX } from "solid-js";
import { lazy, Suspense } from "solid-js";

const load = () => import("./sysinfo-plot");
const LazyTimeChart = lazy(() => load().then((m) => ({ default: m.TimeChart })));
const LazySingleLinePlot = lazy(() => load().then((m) => ({ default: m.SingleLinePlot })));

type TimeChartProps = ComponentProps<typeof import("./sysinfo-plot").TimeChart>;
type SingleLinePlotProps = ComponentProps<typeof import("./sysinfo-plot").SingleLinePlot>;

export function TimeChart(props: TimeChartProps): JSX.Element {
    return (
        <Suspense>
            <LazyTimeChart {...props} />
        </Suspense>
    );
}

export function SingleLinePlot(props: SingleLinePlotProps): JSX.Element {
    return (
        <Suspense>
            <LazySingleLinePlot {...props} />
        </Suspense>
    );
}
