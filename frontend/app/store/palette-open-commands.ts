// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The palette's "Open <pane>" entries that follow the widget bar: Editor, Browser and Drone, and one entry
 * for every other widget (the messengers, a site or a command a person pinned in `widgets.json`). The
 * entry opens what the bar's widget opens: its own `blockdef`, with the singleton rule (`palette-open-pane.ts`).
 * `OPEN_COMMAND_FOR_VIEW` says which command opens each built-in pane view; `command-registry-coverage.test.ts`
 * fails when a registered view is missing from it and has no stated reason.
 */

import { atoms } from "@/app/store/global";
import type { CommandEntry } from "@/app/store/command-registry";
import { openPane } from "@/app/store/palette-open-pane";

/** The palette command that opens each built-in pane view (Shift+Enter makes a new one). */
export const OPEN_COMMAND_FOR_VIEW: Readonly<Record<string, string>> = {
    term: "open:terminal",
    agent: "open:agent",
    sysinfo: "open:sysinfo",
    tower: "open:tower",
    kanban: "open:kanban",
    vault: "open:vault",
    history: "open:history",
    audit: "open:audit",
    warden: "open:jobs",
    help: "open:help",
    reviews: "open:reviews",
    atlas: "open:atlas",
    swarm: "open:swarm",
    files: "open:files",
    connectors: "app:connectors",
    memory: "app:memory",
    settings: "app:settings",
    toolchain: "open:toolchain",
    editor: "open:editor",
    browser: "open:browser",
    drone: "open:drone",
};

interface OpenSpec {
    view: string;
    id: string;
    label: string;
    icon: string;
    keywords: string;
    /** What the widget bar's own entry would open, when the person's `widgets.json` has none. */
    fallback: BlockDef;
}

const OPEN_SPECS: OpenSpec[] = [
    {
        view: "editor",
        id: OPEN_COMMAND_FOR_VIEW.editor,
        label: "Open Editor",
        icon: "file-code",
        keywords: "edit file text code document new file scratch notes",
        fallback: { meta: { view: "editor", "editor:tree_expanded": false, "editor:scratch": true } as MetaType },
    },
    {
        view: "browser",
        id: OPEN_COMMAND_FOR_VIEW.browser,
        label: "Open Browser",
        icon: "globe",
        keywords: "web internet page site url",
        fallback: { meta: { view: "browser" } },
    },
    {
        view: "drone",
        id: OPEN_COMMAND_FOR_VIEW.drone,
        label: "Open Drone (Workflows)",
        icon: "diagram-project",
        keywords: "workflow workflows dag blocks automation",
        fallback: { meta: { view: "drone" } },
    },
];

/** The widgets as the config has them (the person's `widgets.json` over the built-in list). */
function widgets(): Record<string, WidgetConfigType> {
    return atoms.fullConfigAtom()?.widgets ?? {};
}

/** What the bar's widget for `view` opens, so the palette and the bar make the same pane. */
function blockdefFor(spec: OpenSpec): BlockDef {
    return widgets()[`defwidget@${spec.view}`]?.blockdef ?? spec.fallback;
}

/** Editor, Browser and Drone. */
export function openWidgetCommands(): CommandEntry[] {
    return OPEN_SPECS.map((spec) => ({
        id: spec.id,
        label: spec.label,
        category: "Open",
        icon: spec.icon,
        keywords: spec.keywords,
        execute: (opts) => openPane(blockdefFor(spec), opts?.forceNew),
    }));
}

const COVERED_VIEWS: ReadonlySet<string> = new Set(Object.keys(OPEN_COMMAND_FOR_VIEW));

/** A brand or namespaced icon name ("brands@discord") is not a solid icon the palette can draw. */
const plainIcon = (icon: string | undefined): string => (icon && /^[a-z0-9-]+$/.test(icon) ? icon : "up-right-from-square");

/**
 * Whether a widget needs an entry of its own: a site, a terminal command, or a view with no "Open" entry.
 * The Agent widget carries `cmd: ""` (no command), so an empty `cmd` is not a command.
 */
function needsEntry(widget: WidgetConfigType): boolean {
    const meta = widget.blockdef?.meta as Record<string, unknown> | undefined;
    if (!meta) return false;
    const view = meta["view"];
    if (typeof meta["url"] === "string" && meta["url"] !== "") return true;
    if (view === "term" && typeof meta["cmd"] === "string" && meta["cmd"] !== "") return true;
    return typeof view === "string" && !COVERED_VIEWS.has(view);
}

/** One "Open <label>" entry for each widget no other entry opens: Discord, Slack, Telegram, WhatsApp, Teams and the person's own. */
export function widgetEntries(): CommandEntry[] {
    return Object.entries(widgets())
        .filter(([, widget]) => needsEntry(widget))
        .map(([key, widget]) => ({
            id: `open:widget:${key.replace(/^defwidget@/, "")}`,
            label: `Open ${widget.label ?? key}`,
            category: "Open",
            icon: plainIcon(widget.icon),
            iconColor: widget.color,
            keywords: [widget.description, "widget"].filter(Boolean).join(" "),
            execute: (opts) => openPane(widget.blockdef, opts?.forceNew),
        }));
}
