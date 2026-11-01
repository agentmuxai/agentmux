// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Every way into "show this pane or open it" goes through `revealOrOpen`
 * (docs/reports/REPORT_ACTION_RESULTS_ALWAYS_VISIBLE_ACROSS_WINDOWS_2026_10_31.md §4 and §5, the guard): a path that
 * calls `pane.open` or `agent.open` on its own, or looks a pane up by view on its own, skips the window and app
 * scope rules, the verified raise and the notice. This freezes the paths that exist; a new file fails.
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { describe, expect, it } from "vitest";

const FRONTEND_ROOT = join(__dirname, "..", "..");

/**
 * Files that ask srv to open a pane or an agent themselves, and why that is not "reveal or open a view":
 * each places the new pane relative to a pane it already has, floats it, or opens a file, folder, URL or
 * agent rather than a view that is already open.
 */
const OPENS_BY_THEMSELVES: Record<string, string> = {
    "app/element/link-destinations.ts": "a link opens a file, folder or URL, docked or floating",
    "app/store/agent-runtime-live.ts": "opens the named agent's own pane",
    "app/store/open-floating.ts": "the float modifier: always a new floating window",
    "app/util/open-config-file.ts": "opens a config file in the Editor",
    "app/view/agent/agent-open-in-editor.tsx": "opens a file the agent touched, beside the agent",
    "app/view/editor/editor-model.ts": "open to the side / in a terminal, beside its own pane",
    "app/view/editor/viewers/viewer-host.tsx": "opens a URL beside the viewer",
    "app/view/files/files-open.ts": "opens a file or folder from Files, beside it",
    "app/view/remotes/remotes-model.ts": "opens a connection's terminal, files or config, beside Remotes",
    "app/window/action-widgets-menu.ts": "the float modifier on a widget: always a new floating window",
    "layout/lib/layoutStack.ts": "stacks a new tab onto an existing pane",
};

/** The one look-up of "the pane of this view that is already open": revealOrOpen's, and the two lists built on it. */
const LOOKS_UP_BY_VIEW: Record<string, string> = {
    "app/store/block-component-registry.ts": "revealOrOpen's host",
    "app/util/reveal-block.ts": "defines it",
    "app/view/shortcuts/shortcuts.tsx": "lists the panes of a view for the shortcuts page",
};

function sources(dir: string, out: string[] = []): string[] {
    for (const name of readdirSync(dir)) {
        const path = join(dir, name);
        if (statSync(path).isDirectory()) {
            if (name !== "node_modules" && name !== "types") sources(path, out);
        } else if (/\.(ts|tsx)$/.test(name) && !/\.(test|spec)\.|generated/.test(name)) {
            out.push(relative(FRONTEND_ROOT, path).split(sep).join("/"));
        }
    }
    return out;
}

const files = sources(FRONTEND_ROOT);
const using = (pattern: RegExp) => files.filter((f) => pattern.test(readFileSync(join(FRONTEND_ROOT, f), "utf8"))).sort();

describe("the ways into showing or opening a pane", () => {
    it("only the known files ask srv to open a pane or an agent themselves", () => {
        const found = using(/\.(PaneOpenCommand|AgentOpenCommand)\(/);
        expect(
            found,
            "A new file opens a pane or an agent on its own. To bring back the pane of a view that may be open, use openOrFocusPaneByView (revealOrOpen). Otherwise add the file here with the reason.",
        ).toEqual(Object.keys(OPENS_BY_THEMSELVES).sort());
    });

    it("only the known files look a pane up by its view", () => {
        const found = using(/\bfindOpenBlockByView\b/);
        expect(found, "Look a pane up through revealOrOpen, which knows the window and app scope.").toEqual(Object.keys(LOOKS_UP_BY_VIEW).sort());
    });
});
