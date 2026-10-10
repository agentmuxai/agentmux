// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The chrome zoom factor (title bar, status bar, pane headers). A leaf module
// so the layout engine can read it without importing zoom.ts's store and RPC
// dependencies: a minimized pane is a header-sized chip, and the header is
// drawn with `zoom: var(--zoomfactor)`, so the chip's size has to follow this
// value (REPORT_MINIMIZED_PANE_CHROME_ZOOM_DESYNC_2026_10_10.md).
//
// zoom.ts owns writing it (and the `--zoomfactor` CSS variable alongside it).

import { createSignal } from "solid-js";
import { DEFAULT_ZOOM } from "./zoom-factor";

export const [chromeZoomAtom, setChromeZoomSignal] = createSignal<number>(DEFAULT_ZOOM);
