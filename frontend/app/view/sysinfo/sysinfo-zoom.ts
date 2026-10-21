// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createContext } from "solid-js";

/**
 * The pane's zoom (`term:zoom`) for Sysinfo's text: the charts' axis labels and
 * the room they take scale with it, their lines and fills don't. Provided by the
 * view (sysinfo-view.tsx); 1 outside it. Its own module so the view can provide
 * it without loading the charts' code (sysinfo-plot-lazy.tsx).
 */
export const SysinfoZoomContext = createContext<() => number>(() => 1);
