// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A widget's pane whose widget isn't running: removed, waiting for the
 * user's approval, changed since, turned off, invalid, or failed to load
 * (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §8.4). It says
 * which, and opens Settings → Widgets where the user can act on it. Once the
 * widget runs, the pane remounts with it.
 */

import { Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { createBlockSplitHorizontally } from "@/app/store/block-layout-actions";
import { widgetPackages } from "@/app/store/widget-packages-store";
import { widgetLoadErrors } from "./widget-loader";

import "./missing-widget.scss";

export function MissingWidget(props: { blockId: string; view: string }): JSX.Element {
    const packages = widgetPackages();
    const pkg = () => packages().find((p) => p.panes.some((pane) => pane.view === props.view));
    const why = (): string => {
        const p = pkg();
        if (!p) return "This widget isn't installed.";
        switch (p.state) {
            case "needs_approval":
                return `${p.name} is waiting for your approval.`;
            case "changed":
                return `${p.name} changed since you approved it, and waits for you to approve the new version.`;
            case "disabled":
                return `${p.name} is turned off.`;
            case "invalid":
                return `${p.name} can't be loaded: ${p.error ?? "its widget.json has an error"}.`;
            default: {
                const err = widgetLoadErrors()[p.id];
                return err ? `${p.name} didn't load: ${err}` : `Loading ${p.name}…`;
            }
        }
    };
    const openSettings = () =>
        void createBlockSplitHorizontally({ meta: { view: "settings", "settings:section": "widgets" } as MetaType }, props.blockId, "after");
    return (
        <div class="missing-widget" role="status">
            <i class={`fa-solid fa-${pkg()?.icon ?? "puzzle-piece"}`} aria-hidden="true" />
            <div class="missing-widget-text">{why()}</div>
            <Show when={pkg()?.state !== "approved" || widgetLoadErrors()[pkg()?.id ?? ""]}>
                <Button icon="puzzle-piece" onClick={openSettings}>
                    Open Settings → Widgets
                </Button>
            </Show>
        </div>
    );
}
