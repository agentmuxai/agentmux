// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * EmptyPaneLogo — the faint AgentMux brain mark behind an empty pane's own
 * text (an Editor or Media pane with no file open).
 *
 * The parent must be positioned (`relative` or `absolute`); its other children
 * sit above the mark at `position: relative; z-index: 1`. The mark takes no
 * clicks, so a click-to-pick empty state still gets them. Each copy gets
 * its own SVG ids, so two empty panes on screen both show the logo.
 *
 * See docs/specs/SPEC_MEDIA_PANE_EMPTY_LOGO_2026_10_12.md.
 */

import type { JSX } from "solid-js";
import brainSvg from "@/app/asset/logo-brain.svg?raw";
import { uniquifySvgIds } from "@/util/svg-ids";
import "./EmptyPaneLogo.scss";

export const EmptyPaneLogo = (): JSX.Element => (
    <div class="empty-pane-logo" innerHTML={uniquifySvgIds(brainSvg)} aria-hidden="true" />
);
