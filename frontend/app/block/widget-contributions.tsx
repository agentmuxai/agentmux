// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A widget's contributions beyond panes: its command palette entries and its
 * status bar items (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md
 * §6.8). Registered while its package is loaded (`widget-loader.ts`) and
 * removed with it.
 *
 * Running one is a user's click (the palette, a status item): it focuses an
 * open pane of the widget in this tab, or opens one, and delivers the
 * command to it. Agents' RunCommand reaches only the shortcut table, never
 * these.
 */

import { Show, type JSX } from "solid-js";
import { commandRegistry } from "@/app/store/command-registry";
import type { WidgetCommandInfo, WidgetPackageInfo, WidgetStatusItemInfo } from "@/app/store/rpc-api/widgets";
import { registerStatusBarItem, WIDGET_ORDER_LEFT, WIDGET_ORDER_RIGHT } from "@/app/statusbar/status-bar-registry";
import { statusLook, waitForWidgetPane, widgetPanesOf, type CommandSource } from "./widget-panes";

/** How long a pane opened for a command has to load. */
const OPEN_TIMEOUT_MS = 15_000;

export interface ContributionDeps {
    /** The focused pane's block, if any. */
    focusedBlockId(): string | null;
    /** Focuses `blockId` if it is in the active tab (a background pane tab
     *  becomes its pane's visible one); false otherwise. */
    focusPane(blockId: string): boolean | Promise<boolean>;
    /** Opens a pane; resolves to its block id. */
    openPane(blockDef: BlockDef): Promise<string>;
}

const FA_NAME = /^[a-z0-9-]{1,60}$/;
/** A Font Awesome name, or the puzzle piece. */
export const safeIcon = (icon: string | undefined, fallback = "puzzle-piece") => (icon && FA_NAME.test(icon) ? icon : fallback);

/**
 * Runs command `commandId` (null: just show the pane) of `pkg` in a pane of
 * `view`: the focused one, else one in this tab, else a new one. The pane
 * gets the `command` event once it is connected.
 */
export async function runWidgetCommand(
    pkg: WidgetPackageInfo,
    view: string,
    commandId: string | null,
    source: CommandSource,
    deps: ContributionDeps
): Promise<boolean> {
    const open = widgetPanesOf(view);
    const focused = deps.focusedBlockId();
    const candidates = [...open.filter(([b]) => b === focused), ...open.filter(([b]) => b !== focused)];
    let pane = null;
    for (const [blockId, p] of candidates) {
        if (await deps.focusPane(blockId)) {
            pane = p;
            break;
        }
    }
    if (!pane) {
        const info = pkg.panes.find((p) => p.view === view);
        if (!info) return false;
        const blockId = await deps.openPane({ meta: { ...info.default_meta, view } });
        pane = await waitForWidgetPane(blockId, OPEN_TIMEOUT_MS);
        if (!pane) return false;
    }
    if (commandId != null) pane.command?.(commandId, source);
    return true;
}

/** Registers `pkg`'s palette commands and status items; returns what
 *  removes each. */
export function registerWidgetContributions(pkg: WidgetPackageInfo, deps: ContributionDeps): (() => void)[] {
    const out: (() => void)[] = [];
    for (const cmd of pkg.commands ?? []) {
        out.push(
            commandRegistry.register({
                id: `ext:${pkg.id}/${cmd.id}`,
                label: `${pkg.name}: ${cmd.title}`,
                category: "Widgets",
                icon: safeIcon(cmd.icon, safeIcon(pkg.icon)),
                keywords: `${cmd.keywords} widget ${pkg.id}`.trim(),
                execute: () => runWidgetCommand(pkg, cmd.view, cmd.id, "palette", deps),
            })
        );
    }
    (pkg.status_items ?? []).forEach((item, i) => {
        const left = item.alignment === "left";
        out.push(
            registerStatusBarItem({
                id: `widget:${pkg.id}/${item.id}`,
                side: left ? "left" : "right",
                order: (left ? WIDGET_ORDER_LEFT : WIDGET_ORDER_RIGHT) + i,
                render: () => <WidgetStatusItem pkg={pkg} item={item} deps={deps} />,
            })
        );
    });
    return out;
}

/** One widget status item: its manifest's look, or what its pane set. Its
 *  tooltip names the widget, so it can't pass for one of AgentMux's own. */
function WidgetStatusItem(props: { pkg: WidgetPackageInfo; item: WidgetStatusItemInfo; deps: ContributionDeps }): JSX.Element {
    const look = () => statusLook(props.pkg.id, props.item.id);
    const text = () => look()?.text ?? props.item.text;
    const icon = () => safeIcon(look()?.icon ?? props.item.icon, safeIcon(props.pkg.icon));
    const tip = () => `${props.pkg.name}: ${look()?.tooltip ?? props.item.tooltip ?? text()}`;
    const command = (): WidgetCommandInfo | undefined => props.pkg.commands.find((c) => c.id === props.item.command);
    const run = () => {
        const cmd = command();
        void runWidgetCommand(props.pkg, cmd?.view ?? props.pkg.panes[0]?.view ?? "", cmd?.id ?? null, "status", props.deps);
    };
    return (
        <Show when={!look()?.hidden}>
            {/* A status bar readout, like the bar's own (MuxBusIndicator), not a
                line-style control. */}
            <div
                role="button"
                tabIndex={0}
                class={`status-bar-item clickable status-widget-item tone-${look()?.tone ?? "default"}`}
                data-tip={tip()}
                aria-label={tip()}
                onClick={run}
                onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        run();
                    }
                }}
            >
                <i class={`fa fa-solid fa-${icon()} status-icon`} aria-hidden="true" />
                <span>{text()}</span>
            </div>
        </Show>
    );
}
