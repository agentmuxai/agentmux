// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Files pane: a toolbar with history and a breadcrumb, Places
 * on the left, a sortable details list, and a status line.
 * docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §5.
 *
 * The list is windowed by hand (fixed-height rows, §6.5): only the rows in
 * view, plus a few either side, are in the DOM, so a 100,000-entry folder
 * scrolls like a small one. Keyboard follows the ARIA grid pattern with focus
 * separate from selection (§5.3).
 *
 * The setup is a run of hook groups, each in its own file (files-view-*.ts), called in the order their
 * effects and listeners must be created; files-view-parts.tsx draws what they built.
 */

import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import type { JSX } from "solid-js";
import type { FilesModel } from "./files-model";
import "./files.scss";
import { useFilesCommands } from "./files-view-commands";
import { useFilesDrag } from "./files-view-drag";
import { useFilesKeys } from "./files-view-keys";
import { useFilesLifecycle } from "./files-view-lifecycle";
import { useFilesOpening } from "./files-view-opening";
import { FilesViewLayout } from "./files-view-parts";
import { useFilesPointer } from "./files-view-pointer";
import { useFilesLocal } from "./files-view-state";
import { useFilesWindow } from "./files-view-window";

export { formatModified } from "./files-rows";
export { mentionToken } from "./files-agent-mention";
export { errorMessage } from "./files-view-parts";

export function FilesView(props: { model: FilesModel; ctx: PaneTabHostContext }): JSX.Element {
    const model = props.model;
    const ctx = props.ctx;

    const fv = useFilesLocal(model, ctx);
    const drag = useFilesDrag(fv);
    useFilesLifecycle(fv);
    const win = useFilesWindow(fv);
    const opening = useFilesOpening(fv);
    const runFilesCommand = useFilesCommands(fv, win, opening);
    const keys = useFilesKeys(fv, win, opening, runFilesCommand);
    const pointer = useFilesPointer(fv, win, opening);

    return <FilesViewLayout v={{ model, ctx, fv, drag, win, opening, keys, pointer }} />;
}
